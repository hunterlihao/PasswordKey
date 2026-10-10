package main

import (
	"path/filepath"
	"testing"

	"password/internal/crypto"
	"password/internal/export"
	"password/internal/model"
	"password/internal/store"
)

// newTestApp 建一个独立的临时库并把 App 装配好（不依赖 Wails 运行时）。
func newTestApp(t *testing.T) *App {
	t.Helper()
	s, err := store.New(filepath.Join(t.TempDir(), "data.db"))
	if err != nil {
		t.Fatalf("打开测试库失败: %v", err)
	}
	t.Cleanup(func() { s.Close() })

	a := &App{store: s}
	if err := a.SetupMaster("master-pass"); err != nil {
		t.Fatalf("设置主密钥失败: %v", err)
	}
	return a
}

func addOne(t *testing.T, a *App, secret string) int64 {
	t.Helper()
	c := model.Credential{
		Name:   "GitHub",
		Type:   model.TypeBrowserPassword,
		URL:    "https://github.com",
		Secret: secret,
	}
	if err := a.AddCredential(c); err != nil {
		t.Fatalf("新增凭证失败: %v", err)
	}
	views := a.GetCredentials(-1, "")
	if len(views) != 1 {
		t.Fatalf("期望 1 条凭证，实际 %d", len(views))
	}
	return views[0].ID
}

// 回归：编辑凭证时留空密码，已保存的密码必须原样保留。
//
// 曾经的缺陷：前端编辑表单对 secret 字段恒为空值，占位符写明「留空则不修改」，
// 保存时仍把空字符串原样提交（main.js saveCredential）；
// 后端 UpdateCredential 无条件加密提交上来的内容，
// 于是空字符串被加密成密文覆盖掉原有密码 ——
// 表现为「打开看不到密码、复制为空、导出也是空的」。
func TestUpdateCredentialBlankSecretKeepsPassword(t *testing.T) {
	a := newTestApp(t)
	id := addOne(t, a, "P@ssw0rd_Original")

	got, err := a.GetDecryptedSecretDirect(id)
	if err != nil {
		t.Fatalf("新增后解密失败: %v", err)
	}
	if got != "P@ssw0rd_Original" {
		t.Fatalf("新增后密码应为 P@ssw0rd_Original，实际 %q", got)
	}

	before, err := a.store.GetCredential(id)
	if err != nil {
		t.Fatalf("读取原记录失败: %v", err)
	}

	// 模拟用户点「编辑」，只改名称，密码框按提示留空
	if err := a.UpdateCredential(model.Credential{
		ID:     id,
		Name:   "GitHub (改名)",
		Type:   model.TypeBrowserPassword,
		URL:    "https://github.com",
		Secret: "",
	}); err != nil {
		t.Fatalf("更新凭证失败: %v", err)
	}

	after, err := a.store.GetCredential(id)
	if err != nil {
		t.Fatalf("读取更新后记录失败: %v", err)
	}
	if after.Secret != before.Secret {
		t.Fatal("留空密码时不应重写密文")
	}
	if after.Name != "GitHub (改名)" {
		t.Fatalf("名称应被更新，实际 %q", after.Name)
	}

	plain, err := a.GetDecryptedSecretDirect(id)
	if err != nil {
		t.Fatalf("更新后解密失败: %v", err)
	}
	if plain != "P@ssw0rd_Original" {
		t.Fatalf("留空密码后原密码被覆盖：期望 %q，实际 %q", "P@ssw0rd_Original", plain)
	}
}

// 显式传入新密码时必须真正更新（不能被「留空保留」逻辑误伤）。
func TestUpdateCredentialNewSecretStillApplies(t *testing.T) {
	a := newTestApp(t)
	id := addOne(t, a, "old-secret")

	if err := a.UpdateCredential(model.Credential{
		ID:     id,
		Name:   "GitHub",
		Type:   model.TypeBrowserPassword,
		Secret: "new-secret",
	}); err != nil {
		t.Fatalf("更新凭证失败: %v", err)
	}

	got, err := a.GetDecryptedSecretDirect(id)
	if err != nil {
		t.Fatalf("解密失败: %v", err)
	}
	if got != "new-secret" {
		t.Fatalf("显式改密码应生效，实际 %q", got)
	}
}

// 编辑保存时不应把收藏状态清掉：表单固定提交 is_favorite=0，
// 后端必须保留库中原值（收藏只能通过 ToggleFavorite 改变）。
func TestUpdateCredentialKeepsFavorite(t *testing.T) {
	a := newTestApp(t)
	id := addOne(t, a, "secret")

	if err := a.ToggleFavorite(id); err != nil {
		t.Fatalf("切换收藏失败: %v", err)
	}

	// 模拟编辑表单保存：is_favorite 恒为 0
	if err := a.UpdateCredential(model.Credential{
		ID:         id,
		Name:       "GitHub (改名)",
		Type:       model.TypeBrowserPassword,
		Secret:     "",
		IsFavorite: 0,
	}); err != nil {
		t.Fatalf("更新凭证失败: %v", err)
	}

	views := a.GetCredentials(-1, "")
	if len(views) != 1 {
		t.Fatalf("期望 1 条凭证，实际 %d", len(views))
	}
	if views[0].IsFavorite != 1 {
		t.Fatal("编辑保存后收藏状态被清掉了")
	}
}

// 回归：编辑（密码留空）后导出，导出的密码列不能是空的。
// 走的是与 ExportData 相同的数据收集 + 落盘路径（只跳过文件选择对话框）。
func TestExportAfterBlankEditKeepsPassword(t *testing.T) {
	a := newTestApp(t)
	id := addOne(t, a, "P@ssw0rd_Original")

	if err := a.UpdateCredential(model.Credential{
		ID:     id,
		Name:   "GitHub (改名)",
		Type:   model.TypeBrowserPassword,
		Secret: "",
	}); err != nil {
		t.Fatalf("更新凭证失败: %v", err)
	}

	creds, err := a.store.GetAllCredentials()
	if err != nil {
		t.Fatalf("读取凭证失败: %v", err)
	}

	var payload []export.ExportCredential
	if err := a.withKey(func(key []byte) error {
		for _, c := range creds {
			plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
			if err != nil {
				t.Fatalf("导出时解密失败: %v", err)
			}
			payload = append(payload, export.ExportCredentialToExport(c, string(plaintext)))
		}
		return nil
	}); err != nil {
		t.Fatalf("withKey 失败: %v", err)
	}

	out := filepath.Join(t.TempDir(), "export.json")
	if err := export.ExportJSON(payload, out); err != nil {
		t.Fatalf("导出失败: %v", err)
	}

	back, err := export.ImportJSON(out)
	if err != nil {
		t.Fatalf("读回导出文件失败: %v", err)
	}
	if len(back) != 1 {
		t.Fatalf("导出应有 1 条，实际 %d", len(back))
	}
	if back[0].Secret != "P@ssw0rd_Original" {
		t.Fatalf("导出的密码为空或被覆盖：期望 %q，实际 %q", "P@ssw0rd_Original", back[0].Secret)
	}
}
