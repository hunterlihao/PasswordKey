package store

import (
	"database/sql"
	"errors"
	"strings"
	"time"

	"password/internal/model"

	_ "modernc.org/sqlite"
)

const createTablesSQL = `
CREATE TABLE IF NOT EXISTS master_config (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    salt            BLOB NOT NULL,
    encrypted_verify BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS credentials (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    type        INTEGER NOT NULL DEFAULT 0,
    url         TEXT NOT NULL DEFAULT '',
    username    TEXT NOT NULL DEFAULT '',
    secret      BLOB NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    created_at  DATETIME NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at  DATETIME NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS app_state (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    fail_count    INTEGER NOT NULL DEFAULT 0,
    locked_until  TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_credentials_type ON credentials(type);
CREATE INDEX IF NOT EXISTS idx_credentials_name ON credentials(name);
CREATE INDEX IF NOT EXISTS idx_credentials_favorite ON credentials(is_favorite);
`

// escapeLike 转义 SQL LIKE 中的通配符（% _ \），配合 ESCAPE '\' 使用
func escapeLike(s string) string {
	var b strings.Builder
	for _, r := range s {
		switch r {
		case '\\', '%', '_':
			b.WriteByte('\\')
		}
		b.WriteRune(r)
	}
	return b.String()
}

type Store struct {
	db *sql.DB
}

func New(dbPath string) (*Store, error) {
	db, err := sql.Open("sqlite", dbPath+"?_journal_mode=WAL&_busy_timeout=5000")
	if err != nil {
		return nil, err
	}

	if err := db.Ping(); err != nil {
		db.Close()
		return nil, err
	}

	db.SetMaxOpenConns(1)

	s := &Store{db: db}
	if err := s.migrate(); err != nil {
		db.Close()
		return nil, err
	}

	return s, nil
}

func (s *Store) Close() error {
	return s.db.Close()
}

// WithTransaction 在事务中执行 fn，成功时提交，失败时回滚
func (s *Store) WithTransaction(fn func(tx *sql.Tx) error) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}

	if err := fn(tx); err != nil {
		tx.Rollback()
		return err
	}

	return tx.Commit()
}

func (s *Store) migrate() error {
	_, err := s.db.Exec(createTablesSQL)
	return err
}

func (s *Store) GetMasterConfig() (*model.MasterConfig, error) {
	var mc model.MasterConfig
	err := s.db.QueryRow("SELECT id, salt, encrypted_verify FROM master_config WHERE id = 1").Scan(
		&mc.ID, &mc.Salt, &mc.EncryptedVerify,
	)
	if err != nil {
		return nil, err
	}
	return &mc, nil
}

func (s *Store) SetMasterConfig(salt, encryptedVerify []byte) error {
	_, err := s.db.Exec(
		"INSERT OR REPLACE INTO master_config (id, salt, encrypted_verify) VALUES (1, ?, ?)",
		salt, encryptedVerify,
	)
	return err
}

func (s *Store) CreateCredential(c *model.Credential) error {
	now := time.Now()
	result, err := s.db.Exec(
		`INSERT INTO credentials (name, type, url, username, secret, notes, is_favorite, created_at, updated_at)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		c.Name, c.Type, c.URL, c.Username, c.Secret, c.Notes, c.IsFavorite, now, now,
	)
	if err != nil {
		return err
	}

	id, err := result.LastInsertId()
	if err != nil {
		return err
	}
	c.ID = id
	c.CreatedAt = now
	c.UpdatedAt = now
	return nil
}

func (s *Store) GetCredential(id int64) (*model.Credential, error) {
	var c model.Credential
	err := s.db.QueryRow(
		`SELECT id, name, type, url, username, secret, notes, is_favorite, created_at, updated_at
		 FROM credentials WHERE id = ?`, id,
	).Scan(
		&c.ID, &c.Name, &c.Type, &c.URL, &c.Username, &c.Secret, &c.Notes, &c.IsFavorite,
		&c.CreatedAt, &c.UpdatedAt,
	)
	if err != nil {
		return nil, err
	}
	return &c, nil
}

func (s *Store) UpdateCredential(c *model.Credential) error {
	now := time.Now()
	result, err := s.db.Exec(
		`UPDATE credentials SET name=?, type=?, url=?, username=?, secret=?, notes=?, is_favorite=?, updated_at=?
		 WHERE id=?`,
		c.Name, c.Type, c.URL, c.Username, c.Secret, c.Notes, c.IsFavorite, now, c.ID,
	)
	if err != nil {
		return err
	}
	rows, _ := result.RowsAffected()
	if rows == 0 {
		return errors.New("凭证不存在")
	}
	c.UpdatedAt = now
	return nil
}

func (s *Store) DeleteCredential(id int64) error {
	result, err := s.db.Exec("DELETE FROM credentials WHERE id = ?", id)
	if err != nil {
		return err
	}
	rows, _ := result.RowsAffected()
	if rows == 0 {
		return errors.New("凭证不存在")
	}
	return nil
}

func (s *Store) DeleteCredentials(ids []int64) error {
	if len(ids) == 0 {
		return nil
	}
	return s.WithTransaction(func(tx *sql.Tx) error {
		for _, id := range ids {
			if _, err := tx.Exec("DELETE FROM credentials WHERE id = ?", id); err != nil {
				return err
			}
		}
		return nil
	})
}

func (s *Store) ListCredentials(credType *model.CredentialType, keyword string) ([]model.Credential, error) {
	query := `SELECT id, name, type, url, username, secret, notes, is_favorite, created_at, updated_at
		      FROM credentials WHERE 1=1`
	args := []interface{}{}

	if credType != nil {
		query += " AND type = ?"
		args = append(args, *credType)
	}

	if keyword != "" {
		query += " AND (name LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\' OR username LIKE ? ESCAPE '\\')"
		kw := "%" + escapeLike(keyword) + "%"
		args = append(args, kw, kw, kw)
	}

	query += " ORDER BY is_favorite DESC, updated_at DESC"

	rows, err := s.db.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var credentials []model.Credential
	for rows.Next() {
		var c model.Credential
		err := rows.Scan(
			&c.ID, &c.Name, &c.Type, &c.URL, &c.Username, &c.Secret, &c.Notes, &c.IsFavorite,
			&c.CreatedAt, &c.UpdatedAt,
		)
		if err != nil {
			return nil, err
		}
		credentials = append(credentials, c)
	}

	return credentials, rows.Err()
}

func (s *Store) ToggleFavorite(id int64) error {
	now := time.Now()
	result, err := s.db.Exec(
		"UPDATE credentials SET is_favorite = 1 - is_favorite, updated_at = ? WHERE id = ?",
		now, id,
	)
	if err != nil {
		return err
	}
	rows, _ := result.RowsAffected()
	if rows == 0 {
		return errors.New("凭证不存在")
	}
	return nil
}

func (s *Store) GetAllCredentials() ([]model.Credential, error) {
	rows, err := s.db.Query(
		`SELECT id, name, type, url, username, secret, notes, is_favorite, created_at, updated_at
		 FROM credentials ORDER BY id`,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var credentials []model.Credential
	for rows.Next() {
		var c model.Credential
		err := rows.Scan(
			&c.ID, &c.Name, &c.Type, &c.URL, &c.Username, &c.Secret, &c.Notes, &c.IsFavorite,
			&c.CreatedAt, &c.UpdatedAt,
		)
		if err != nil {
			return nil, err
		}
		credentials = append(credentials, c)
	}
	return credentials, rows.Err()
}

// CountByType 返回各类型凭证数量，key -1=全部, -2=收藏
func (s *Store) CountByType() (map[int]int, error) {
	counts := make(map[int]int)

	var total int
	if err := s.db.QueryRow("SELECT COUNT(*) FROM credentials").Scan(&total); err != nil {
		return nil, err
	}
	counts[-1] = total

	var fav int
	if err := s.db.QueryRow("SELECT COUNT(*) FROM credentials WHERE is_favorite = 1").Scan(&fav); err != nil {
		return nil, err
	}
	counts[-2] = fav

	rows, err := s.db.Query("SELECT type, COUNT(*) FROM credentials GROUP BY type")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	for rows.Next() {
		var t int
		var cnt int
		if err := rows.Scan(&t, &cnt); err != nil {
			return nil, err
		}
		counts[t] = cnt
	}
	return counts, rows.Err()
}

func (s *Store) ListFavorites(keyword string) ([]model.Credential, error) {
	query := `SELECT id, name, type, url, username, secret, notes, is_favorite, created_at, updated_at
		      FROM credentials WHERE is_favorite = 1`
	args := []interface{}{}

	if keyword != "" {
		query += " AND (name LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\' OR username LIKE ? ESCAPE '\\')"
		kw := "%" + escapeLike(keyword) + "%"
		args = append(args, kw, kw, kw)
	}

	query += " ORDER BY updated_at DESC"

	rows, err := s.db.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var credentials []model.Credential
	for rows.Next() {
		var c model.Credential
		err := rows.Scan(
			&c.ID, &c.Name, &c.Type, &c.URL, &c.Username, &c.Secret, &c.Notes, &c.IsFavorite,
			&c.CreatedAt, &c.UpdatedAt,
		)
		if err != nil {
			return nil, err
		}
		credentials = append(credentials, c)
	}

	return credentials, rows.Err()
}

// UnlockState 持久化的暴力破解防护状态
type UnlockState struct {
	FailCount   int
	LockedUntil time.Time // 零值表示未锁定
}

// GetUnlockState 读取持久化的解锁失败状态
func (s *Store) GetUnlockState() (*UnlockState, error) {
	var st UnlockState
	var lockedStr string
	err := s.db.QueryRow(
		"SELECT fail_count, locked_until FROM app_state WHERE id = 1",
	).Scan(&st.FailCount, &lockedStr)
	if err == sql.ErrNoRows {
		return &UnlockState{}, nil
	}
	if err != nil {
		return nil, err
	}
	if lockedStr != "" {
		if t, err := time.Parse("2006-01-02 15:04:05", lockedStr); err == nil {
			st.LockedUntil = t
		}
	}
	return &st, nil
}

// SetUnlockState 持久化解锁失败状态
func (s *Store) SetUnlockState(st *UnlockState) error {
	var lockedStr string
	if !st.LockedUntil.IsZero() {
		lockedStr = st.LockedUntil.Format("2006-01-02 15:04:05")
	}
	_, err := s.db.Exec(
		"INSERT OR REPLACE INTO app_state (id, fail_count, locked_until) VALUES (1, ?, ?)",
		st.FailCount, lockedStr,
	)
	return err
}
