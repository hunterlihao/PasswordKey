package model

import "time"

type CredentialType int

const (
	TypeBrowserPassword CredentialType = iota // 0 - 浏览器密码
	TypeAPIKey                                 // 1 - API Key
	TypeSSHKey                                 // 2 - SSH 密钥
	TypeSecureNote                             // 3 - 安全笔记
	TypeDatabase                               // 4 - 数据库连接
	TypeEmail                                  // 5 - 邮箱账号
	TypeBankCard                               // 6 - 银行卡/信用卡
)

// Credential 用于数据库存储和前端输入。
// 注意: Secret 存储加密后的密文。列表查询通过 ToView() 转换为 CredentialView，
// 避免将 Secret 暴露给前端列表接口。AddCredential/UpdateCredential 通过此结构体接收前端传入的明文。
type Credential struct {
	ID         int64          `json:"id"`
	Name       string         `json:"name"`
	Type       CredentialType `json:"type"`
	URL        string         `json:"url"`
	Username   string         `json:"username"`
	Secret     string         `json:"secret"`
	Notes      string         `json:"notes"`
	IsFavorite int            `json:"is_favorite"`
	CreatedAt  time.Time      `json:"created_at"`
	UpdatedAt  time.Time      `json:"updated_at"`
}

// CredentialView 用于前端展示，不包含 Secret 密文
type CredentialView struct {
	ID         int64          `json:"id"`
	Name       string         `json:"name"`
	Type       CredentialType `json:"type"`
	URL        string         `json:"url"`
	Username   string         `json:"username"`
	Notes      string         `json:"notes"`
	IsFavorite int            `json:"is_favorite"`
	CreatedAt  time.Time      `json:"created_at"`
	UpdatedAt  time.Time      `json:"updated_at"`
}

func (c Credential) ToView() CredentialView {
	return CredentialView{
		ID:         c.ID,
		Name:       c.Name,
		Type:       c.Type,
		URL:        c.URL,
		Username:   c.Username,
		Notes:      c.Notes,
		IsFavorite: c.IsFavorite,
		CreatedAt:  c.CreatedAt,
		UpdatedAt:  c.UpdatedAt,
	}
}

type MasterConfig struct {
	ID              int    `json:"id"`
	Salt            []byte `json:"salt"`
	EncryptedVerify []byte `json:"encrypted_verify"`
}

type PasswordStrength int

const (
	StrengthWeak PasswordStrength = iota
	StrengthMedium
	StrengthStrong
)

func CheckPasswordStrength(password string) PasswordStrength {
	score := 0
	length := len(password)

	if length >= 8 {
		score++
	}
	if length >= 12 {
		score++
	}
	if length >= 16 {
		score++
	}

	hasLower, hasUpper, hasDigit, hasSpecial := false, false, false, false
	for _, c := range password {
		switch {
		case c >= 'a' && c <= 'z':
			hasLower = true
		case c >= 'A' && c <= 'Z':
			hasUpper = true
		case c >= '0' && c <= '9':
			hasDigit = true
		default:
			hasSpecial = true
		}
	}

	if hasLower {
		score++
	}
	if hasUpper {
		score++
	}
	if hasDigit {
		score++
	}
	if hasSpecial {
		score++
	}

	switch {
	case score >= 6:
		return StrengthStrong
	case score >= 4:
		return StrengthMedium
	default:
		return StrengthWeak
	}
}
