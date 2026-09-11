package settings

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

const settingsFile = "settings.json"

type Settings struct {
	DbPath string `json:"db_path,omitempty"`
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
