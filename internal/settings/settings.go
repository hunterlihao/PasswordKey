package settings

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

const settingsFile = "settings.json"

// DefaultAutoLockMinutes 默认自动锁定时长（分钟）。
// 旧版配置文件没有该字段时回落到此值。
const DefaultAutoLockMinutes = 5

type Settings struct {
	DbPath string `json:"db_path,omitempty"`
	// AutoLockMinutes 闲置自动锁定时长（分钟）。0 表示永不自动锁定。
	// 用指针以区分「未设置」（nil，回落默认值）与「显式设为 0」（永不锁定）。
	AutoLockMinutes *int `json:"auto_lock_minutes,omitempty"`
}

// EffectiveAutoLockMinutes 返回生效的自动锁定时长（分钟）。
// 未设置时返回默认值；显式设为 0 表示永不锁定。
func (s *Settings) EffectiveAutoLockMinutes() int {
	if s == nil || s.AutoLockMinutes == nil {
		return DefaultAutoLockMinutes
	}
	return *s.AutoLockMinutes
}

func Load(dir string) (*Settings, error) {
	path := filepath.Join(dir, settingsFile)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Settings{}, nil
		}
		return nil, err
	}

	var s Settings
	if err := json.Unmarshal(data, &s); err != nil {
		// 配置文件损坏时报错，避免静默返回空配置导致误判 dbPath 为空、
		// 进而引导用户重新设置主密钥、覆盖原有自定义路径的数据库
		return nil, fmt.Errorf("配置文件解析失败（已损坏）: %w", err)
	}
	return &s, nil
}

func (s *Settings) Save(dir string) error {
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, settingsFile), data, 0600)
}
