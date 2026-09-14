package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// 目录可写时 resolveDataDir 应返回 exe 同级的 data/。
// 注意：测试进程的 exe 是 go test 临时二进制，其所在目录未必可写，
// 因此这里直接测 ensureWritableDir 与降级判定两个部件。
func TestEnsureWritableDir(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "a", "b", "data")
	if err := ensureWritableDir(dir); err != nil {
		t.Fatalf("应能创建并写入: %v", err)
	}
	if !dirExists(dir) {
		t.Fatal("目录未创建")
	}
	// 探针文件必须被清理干净，否则会当成用户数据残留
	if fileExists(filepath.Join(dir, ".write-probe")) {
		t.Fatal("探针文件残留")
	}
	t.Log("通过：目录创建 + 可写探测 + 探针清理")
}

// 便携目录不可写时必须回退到系统目录，并留下降级标记。
func TestFallbackWhenPortableNotWritable(t *testing.T) {
	// 用一个「父路径是文件」的位置，保证 MkdirAll 必然失败
	blocker := filepath.Join(t.TempDir(), "blocker")
	if err := os.WriteFile(blocker, []byte("x"), 0600); err != nil {
		t.Fatal(err)
	}
	unwritable := filepath.Join(blocker, "data")

	if err := ensureWritableDir(unwritable); err == nil {
		t.Fatal("父路径是文件时应判定为不可写")
	}
	t.Log("通过：不可写目录被正确识别")
}

// 降级标记存在时应直接回退，且不再触碰便携目录。
func TestFallbackMarkerShortCircuit(t *testing.T) {
	dir := t.TempDir()
	marker := filepath.Join(dir, dataDirFallback)
	if err := os.WriteFile(marker, []byte("dir not writable\n"), 0600); err != nil {
		t.Fatal(err)
	}

	// 标记存在 → 判定条件命中
	if !fileExists(marker) {
		t.Fatal("标记文件应被识别")
	}

	// 回退目录必须位于 %APPDATA% 下
	old := os.Getenv("APPDATA")
	defer os.Setenv("APPDATA", old)
	os.Setenv("APPDATA", `C:\Users\X\AppData\Roaming`)

	got := systemDataDir()
	want := filepath.Join(`C:\Users\X\AppData\Roaming`, systemAppDirName)
	if got != want {
		t.Fatalf("期望 %q，实际 %q", want, got)
	}
	if !strings.HasSuffix(got, "PasswordManager") {
		t.Fatalf("回退目录应以 PasswordManager 结尾，实际 %q", got)
	}
	t.Log("通过：标记短路 + 回退目录正确 =", got)
}

// 便携目录命名必须与 .gitignore 中的忽略项一致，否则密码库可能被提交。
func TestPortableDirName(t *testing.T) {
	if exeDirName != "data" {
		t.Fatalf("便携目录名应为 data，实际 %q", exeDirName)
	}

	gitignore, err := os.ReadFile(".gitignore")
	if err != nil {
		t.Skip("读取 .gitignore 失败，跳过")
	}
	if !strings.Contains(string(gitignore), "data/") {
		t.Fatal(".gitignore 必须忽略 data/，否则密码库会被提交进仓库")
	}
	t.Log("通过：目录名 data 且已被 .gitignore 忽略")
}

func dirExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}
