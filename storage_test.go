package main

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

// 场景 1：便携目录为空 + 旧位置有数据 → 应完整迁移（含 -wal）
func TestMigrateFromLegacy(t *testing.T) {
	root := t.TempDir()
	portable := filepath.Join(root, "exe", "data")
	legacy := filepath.Join(root, "appdata", "PasswordManager")
	mustMkdir(t, legacy)

	writeFile(t, filepath.Join(legacy, "settings.json"), `{"db_path":"","auto_lock_minutes":10}`)
	writeFile(t, filepath.Join(legacy, "data.db"), "SQLITE-FAKE-MAIN")
	writeFile(t, filepath.Join(legacy, "data.db-wal"), "WAL-CONTENT")

	mustMkdir(t, portable)
	if err := migrateLegacyData(portable, legacy); err != nil {
		t.Fatalf("迁移失败: %v", err)
	}

	assertContent(t, filepath.Join(portable, "settings.json"), `{"db_path":"","auto_lock_minutes":10}`)
	assertContent(t, filepath.Join(portable, "data.db"), "SQLITE-FAKE-MAIN")
	assertContent(t, filepath.Join(portable, "data.db-wal"), "WAL-CONTENT")
	t.Log("场景1 通过：配置 + 主库 + WAL 均已迁移")
}

// 场景 2：便携目录已有数据 → 必须原样保留，不被旧数据覆盖
func TestMigrateDoesNotOverwriteExisting(t *testing.T) {
	root := t.TempDir()
	portable := filepath.Join(root, "exe", "data")
	legacy := filepath.Join(root, "appdata", "PasswordManager")
	mustMkdir(t, portable)
	mustMkdir(t, legacy)

	writeFile(t, filepath.Join(portable, "settings.json"), `{"auto_lock_minutes":30}`)
	writeFile(t, filepath.Join(legacy, "settings.json"), `{"auto_lock_minutes":1}`)
	writeFile(t, filepath.Join(legacy, "data.db"), "OLD-DB")

	if err := migrateLegacyData(portable, legacy); err != nil {
		t.Fatalf("迁移失败: %v", err)
	}

	assertContent(t, filepath.Join(portable, "settings.json"), `{"auto_lock_minutes":30}`)
	if fileExists(filepath.Join(portable, "data.db")) {
		t.Fatal("便携目录已有数据时不应迁入 data.db")
	}
	t.Log("场景2 通过：便携数据未被覆盖")
}

// 场景 3：旧位置不存在 → 静默跳过，不报错
func TestMigrateNoLegacy(t *testing.T) {
	root := t.TempDir()
	portable := filepath.Join(root, "exe", "data")
	mustMkdir(t, portable)

	if err := migrateLegacyData(portable, filepath.Join(root, "nope")); err != nil {
		t.Fatalf("旧位置不存在时不应报错: %v", err)
	}
	entries, _ := os.ReadDir(portable)
	if len(entries) != 0 {
		t.Fatalf("不应产生任何文件，实际: %v", entries)
	}
	t.Log("场景3 通过：无旧数据时静默跳过")
}

// 场景 4：便携目录可写判定
func TestEnsureWritableDir(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "a", "b", "data")
	if err := ensureWritableDir(dir); err != nil {
		t.Fatalf("应能创建并写入: %v", err)
	}
	if !dirExists(dir) {
		t.Fatal("目录未创建")
	}
	// 探针文件必须被清理干净
	if fileExists(filepath.Join(dir, ".write-probe")) {
		t.Fatal("探针文件残留")
	}
	t.Log("场景4 通过：目录创建 + 可写探测 + 探针清理")
}

// 场景 5：wraparound 无关，但校验 fallbackDataDir 尊重 APPDATA
func TestFallbackDataDir(t *testing.T) {
	old := os.Getenv("APPDATA")
	defer os.Setenv("APPDATA", old)
	os.Setenv("APPDATA", `C:\Users\X\AppData\Roaming`)

	got, err := fallbackDataDir()
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(`C:\Users\X\AppData\Roaming`, "PasswordManager")
	if got != want {
		t.Fatalf("期望 %q，实际 %q", want, got)
	}
	fmt.Println("fallback =", got)
	t.Log("场景5 通过：回退目录基于 APPDATA")
}

func mustMkdir(t *testing.T, dir string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0600); err != nil {
		t.Fatal(err)
	}
}

func assertContent(t *testing.T, path, want string) {
	t.Helper()
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("读取 %s 失败: %v", path, err)
	}
	if string(got) != want {
		t.Fatalf("%s 内容不符，期望 %q 实际 %q", path, want, string(got))
	}
}
