package crypto

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"errors"
	"io"
	"math/big"

	"golang.org/x/crypto/argon2"
)

const (
	SaltLength  = 16
	KeyLength   = 32
	TimeCost    = 3
	MemoryCost  = 64 * 1024
	Parallelism = 4
)

func GenerateSalt() ([]byte, error) {
	salt := make([]byte, SaltLength)
	if _, err := io.ReadFull(rand.Reader, salt); err != nil {
		return nil, err
	}
	return salt, nil
}

func DeriveKey(password string, salt []byte) []byte {
	return argon2.IDKey([]byte(password), salt, TimeCost, MemoryCost, Parallelism, KeyLength)
}

func Encrypt(key []byte, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}

	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}

	nonce := make([]byte, gcm.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return nil, err
	}

	return gcm.Seal(nonce, nonce, plaintext, nil), nil
}

func Decrypt(key []byte, ciphertext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}

	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}

	nonceSize := gcm.NonceSize()
	if len(ciphertext) < nonceSize {
		return nil, errors.New("密文长度不足")
	}

	nonce, ciphertext := ciphertext[:nonceSize], ciphertext[nonceSize:]
	return gcm.Open(nil, nonce, ciphertext, nil)
}

func GenerateVerifyToken() ([]byte, error) {
	token := make([]byte, 32)
	if _, err := io.ReadFull(rand.Reader, token); err != nil {
		return nil, err
	}
	return token, nil
}

func GenerateRandomPassword(length int, useUpper, useDigit, useSpecial bool) (string, error) {
	lower := "abcdefghijklmnopqrstuvwxyz"
	upper := "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
	digits := "0123456789"
	special := "!@#$%^&*()_+-=[]{}|;:,.<>?"

	charset := lower
	if useUpper {
		charset += upper
	}
	if useDigit {
		charset += digits
	}
	if useSpecial {
		charset += special
	}

	if length < 4 {
		length = 4
	}

	password := make([]byte, length)
	for i := range password {
		idx, err := rand.Int(rand.Reader, big.NewInt(int64(len(charset))))
		if err != nil {
			return "", err
		}
		password[i] = charset[idx.Int64()]
	}

	// 保证每种启用的字符类别至少出现一次
	pos := 0
	ensureChars := func(s string) error {
		if pos >= length {
			return nil
		}
		idx, err := rand.Int(rand.Reader, big.NewInt(int64(len(s))))
		if err != nil {
			return err
		}
		password[pos] = s[idx.Int64()]
		pos++
		return nil
	}

	if err := ensureChars(lower); err != nil {
		return "", err
	}
	if useUpper {
		if err := ensureChars(upper); err != nil {
			return "", err
		}
	}
	if useDigit {
		if err := ensureChars(digits); err != nil {
			return "", err
		}
	}
	if useSpecial {
		if err := ensureChars(special); err != nil {
			return "", err
		}
	}

	// 打乱顺序
	for i := len(password) - 1; i > 0; i-- {
		j, err := rand.Int(rand.Reader, big.NewInt(int64(i+1)))
		if err != nil {
			continue
		}
		password[i], password[j.Int64()] = password[j.Int64()], password[i]
	}

	return string(password), nil
}
