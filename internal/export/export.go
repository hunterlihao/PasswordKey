package export

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"password/internal/model"

	"github.com/xuri/excelize/v2"
)

type ExportCredential struct {
	Name     string `json:"name"`
	Type     string `json:"type"`
	URL      string `json:"url"`
	Username string `json:"username"`
	Secret   string `json:"secret"`
	Notes    string `json:"notes"`
}

func credTypeStr(t model.CredentialType) string {
	switch t {
	case model.TypeAPIKey:
		return "api_key"
	case model.TypeSSHKey:
		return "ssh_key"
	case model.TypeSecureNote:
		return "secure_note"
	case model.TypeDatabase:
		return "database"
	case model.TypeEmail:
		return "email"
	case model.TypeBankCard:
		return "bank_card"
	default:
		return "browser"
	}
}

func parseCredType(s string) model.CredentialType {
	switch s {
	case "api_key", "API Key":
		return model.TypeAPIKey
	case "ssh_key":
		return model.TypeSSHKey
	case "secure_note":
		return model.TypeSecureNote
	case "database":
		return model.TypeDatabase
	case "email":
		return model.TypeEmail
	case "bank_card":
		return model.TypeBankCard
	default:
		return model.TypeBrowserPassword
	}
}

// ExportJSON 导出为JSON
func ExportJSON(credentials []ExportCredential, filePath string) error {
	data, err := json.MarshalIndent(credentials, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filePath, data, 0600)
}

// ExportExcel 导出为Excel
func ExportExcel(credentials []ExportCredential, filePath string) error {
	f := excelize.NewFile()
	defer f.Close()

	sheet := "Sheet1"
	f.SetCellValue(sheet, "A1", "name")
	f.SetCellValue(sheet, "B1", "type")
	f.SetCellValue(sheet, "C1", "url")
	f.SetCellValue(sheet, "D1", "username")
	f.SetCellValue(sheet, "E1", "secret")
	f.SetCellValue(sheet, "F1", "notes")

	for i, c := range credentials {
		row := i + 2
		f.SetCellValue(sheet, fmt.Sprintf("A%d", row), c.Name)
		f.SetCellValue(sheet, fmt.Sprintf("B%d", row), c.Type)
		f.SetCellValue(sheet, fmt.Sprintf("C%d", row), c.URL)
		f.SetCellValue(sheet, fmt.Sprintf("D%d", row), c.Username)
		f.SetCellValue(sheet, fmt.Sprintf("E%d", row), c.Secret)
		f.SetCellValue(sheet, fmt.Sprintf("F%d", row), c.Notes)
	}

	if err := f.SaveAs(filePath); err != nil {
		return err
	}
	return os.Chmod(filePath, 0600)
}

// ImportJSON 从JSON导入
func ImportJSON(filePath string) ([]ExportCredential, error) {
	data, err := os.ReadFile(filePath)
	if err != nil {
		return nil, err
	}

	var credentials []ExportCredential
	if err := json.Unmarshal(data, &credentials); err != nil {
		return nil, err
	}

	return credentials, nil
}

// ImportExcel 从Excel导入
func ImportExcel(filePath string) ([]ExportCredential, error) {
	f, err := excelize.OpenFile(filePath)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	sheet := f.GetSheetName(0)
	rows, err := f.GetRows(sheet)
	if err != nil {
		return nil, err
	}

	var credentials []ExportCredential
	for i, row := range rows {
		if i == 0 {
			continue
		}
		getCell := func(idx int) string {
			if idx < len(row) {
				return row[idx]
			}
			return ""
		}
		cred := ExportCredential{
			Name:     getCell(0),
			Type:     getCell(1),
			URL:      getCell(2),
			Username: getCell(3),
			Secret:   getCell(4),
			Notes:    getCell(5),
		}
		if cred.Name != "" && cred.Secret != "" {
			credentials = append(credentials, cred)
		}
	}

	return credentials, nil
}

// ExportCredentialToExport 转换为导出格式
func ExportCredentialToExport(c model.Credential, secret string) ExportCredential {
	return ExportCredential{
		Name:     c.Name,
		Type:     credTypeStr(c.Type),
		URL:      c.URL,
		Username: c.Username,
		Secret:   secret,
		Notes:    c.Notes,
	}
}

// ExportCredentialToModel 转换为数据库模型
func ExportCredentialToModel(ec ExportCredential) model.Credential {
	return model.Credential{
		Name:     ec.Name,
		Type:     parseCredType(ec.Type),
		URL:      ec.URL,
		Username: ec.Username,
		Secret:   ec.Secret,
		Notes:    ec.Notes,
	}
}

// JSONTemplate 生成JSON模板内容
func JSONTemplate() ([]byte, error) {
	template := []ExportCredential{
		{Name: "GitHub", Type: "browser", URL: "https://github.com", Username: "user@example.com", Secret: "your_password_here", Notes: "备注信息"},
		{Name: "OpenAI", Type: "api_key", URL: "", Username: "", Secret: "sk-xxxxxxxxxxxxxxxxxxxx", Notes: "API Key 备注"},
		{Name: "生产服务器", Type: "ssh_key", URL: "192.168.1.100", Username: "root", Secret: "-----BEGIN OPENSSH PRIVATE KEY-----\n...\n-----END OPENSSH PRIVATE KEY-----", Notes: "端口 22"},
		{Name: "WiFi密码", Type: "secure_note", URL: "", Username: "", Secret: "MyWiFi@2024", Notes: "客厅路由器密码"},
		{Name: "MySQL", Type: "database", URL: "localhost:3306", Username: "admin", Secret: "db_password", Notes: "生产数据库"},
		{Name: "Gmail", Type: "email", URL: "smtp.gmail.com", Username: "user@gmail.com", Secret: "app_password", Notes: "IMAP: imap.gmail.com"},
		{Name: "招商银行", Type: "bank_card", URL: "", Username: "6225 **** **** 1234", Secret: "***", Notes: "储蓄卡，持卡人: 张三"},
	}
	data, err := json.MarshalIndent(template, "", "  ")
	if err != nil {
		return nil, err
	}
	return data, nil
}

// ExcelTemplate 生成Excel模板文件
func ExcelTemplate(filePath string) error {
	f := excelize.NewFile()
	defer f.Close()

	sheet := "Sheet1"
	f.SetCellValue(sheet, "A1", "name")
	f.SetCellValue(sheet, "B1", "type")
	f.SetCellValue(sheet, "C1", "url")
	f.SetCellValue(sheet, "D1", "username")
	f.SetCellValue(sheet, "E1", "secret")
	f.SetCellValue(sheet, "F1", "notes")

	f.SetCellValue(sheet, "A2", "GitHub")
	f.SetCellValue(sheet, "B2", "browser")
	f.SetCellValue(sheet, "C2", "https://github.com")
	f.SetCellValue(sheet, "D2", "user@example.com")
	f.SetCellValue(sheet, "E2", "your_password_here")
	f.SetCellValue(sheet, "F2", "备注信息")

	f.SetCellValue(sheet, "A3", "OpenAI")
	f.SetCellValue(sheet, "B3", "api_key")
	f.SetCellValue(sheet, "C3", "")
	f.SetCellValue(sheet, "D3", "")
	f.SetCellValue(sheet, "E3", "sk-xxxxxxxxxxxxxxxxxxxx")
	f.SetCellValue(sheet, "F3", "API Key 备注")

	f.SetCellValue(sheet, "A4", "生产服务器")
	f.SetCellValue(sheet, "B4", "ssh_key")
	f.SetCellValue(sheet, "C4", "192.168.1.100")
	f.SetCellValue(sheet, "D4", "root")
	f.SetCellValue(sheet, "E4", "私钥内容")
	f.SetCellValue(sheet, "F4", "端口 22")

	f.SetCellValue(sheet, "A5", "MySQL")
	f.SetCellValue(sheet, "B5", "database")
	f.SetCellValue(sheet, "C5", "localhost:3306")
	f.SetCellValue(sheet, "D5", "admin")
	f.SetCellValue(sheet, "E5", "db_password")
	f.SetCellValue(sheet, "F5", "生产数据库")

	f.SetCellValue(sheet, "A6", "Gmail")
	f.SetCellValue(sheet, "B6", "email")
	f.SetCellValue(sheet, "C6", "smtp.gmail.com")
	f.SetCellValue(sheet, "D6", "user@gmail.com")
	f.SetCellValue(sheet, "E6", "app_password")
	f.SetCellValue(sheet, "F6", "IMAP: imap.gmail.com")

	if err := f.SaveAs(filePath); err != nil {
		return err
	}
	return os.Chmod(filePath, 0600)
}

// GuessFormat 从文件扩展名判断格式
func GuessFormat(filePath string) string {
	ext := strings.ToLower(filepath.Ext(filePath))
	if ext == ".xlsx" {
		return "excel"
	}
	return "json"
}
