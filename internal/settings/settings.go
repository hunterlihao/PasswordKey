package settings

import (
	"encoding/json"
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
		return &Settings{}, nil
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
