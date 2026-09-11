package model

import (
	"math"
	"time"
)

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
	// 基于熵估计的强度判定：根据实际使用的字符集大小计算 password 的最大熵
	// log2(poolSize) * length，再结合长度门槛分级
	const (
		entropyWeak   = 40.0  // < 40 bits
		entropyMedium = 60.0  // 40 ~ 60 bits
		entropyStrong = 80.0  // >= 80 bits 为强
	)

	hasLower, hasUpper, hasDigit, hasSpecialASCII, hasOther := false, false, false, false, false
	for _, r := range password {
		switch {
		case r >= 'a' && r <= 'z':
			hasLower = true
		case r >= 'A' && r <= 'Z':
			hasUpper = true
		case r >= '0' && r <= '9':
			hasDigit = true
		case r >= 0x21 && r <= 0x7E:
			// 可打印 ASCII 标点（排除字母数字）
			hasSpecialASCII = true
		default:
			// 非 ASCII 字符（含中文等）统一计入一个"其他"池
			hasOther = true
		}
	}

	poolSize := 0
	if hasLower {
		poolSize += 26
	}
	if hasUpper {
		poolSize += 26
	}
	if hasDigit {
		poolSize += 10
	}
	if hasSpecialASCII {
		poolSize += 33
	}
	if hasOther {
		// 非 ASCII 字符池至少按 94 估算（保守）
		poolSize += 94
	}

	length := len(password)
	if poolSize == 0 || length == 0 {
		return StrengthWeak
	}

	// 单字符熵
	bitsPerChar := math.Log2(float64(poolSize))
	entropy := bitsPerChar * float64(length)

	switch {
	case entropy >= entropyStrong:
		return StrengthStrong
	case entropy >= entropyMedium:
		return StrengthMedium
	case entropy >= entropyWeak:
		// 达到弱门槛但低于中门槛，再按短密码进一步压制
		if length < 8 {
			return StrengthWeak
		}
		return StrengthMedium
	default:
		return StrengthWeak
	}
}
