package main

import (
	"crypto/sha256"
	"errors"
	"sort"

	"password/internal/crypto"
	"password/internal/model"
)

// GetAuditReport 生成安全体检报告：弱密码 + 重复密码。
//
// 安全性要点：审计必须解密每条密码才能判定强度与是否重复，
// 因此全程在内存中处理明文，且：
//   - 明文只在单个循环体内短暂存活，用完立即覆盖清零；
//   - 重复检测用 SHA-256 摘要分组，不保留明文副本、也不用明文做 map key；
//   - 报告只返回掩码（MaskSecret），绝不把明文或摘要交给前端。
//
// 已知局限：model.Credential.Secret 是 string 类型，Go 的 string 不可变
// 因而无法原地清零，`[]byte(c.Secret)` 还会额外复制一份到不可回收区。
// 也就是说本函数读取密文时会产生无法擦除的临时副本。彻底解决需要把
// Secret 改成 []byte（属既有的 P1 整改项，涉及面较大，此处不动）。
//
// 必须在解锁状态下调用，未解锁直接返回错误。
func (a *App) GetAuditReport() (*model.AuditReport, error) {
	if a.store == nil {
		return nil, errors.New("数据库未初始化")
	}

	report := &model.AuditReport{
		Weak:            []model.AuditItem{},
		DuplicateGroups: []model.DuplicateGroup{},
	}

	var dupGroups []model.DuplicateGroup

	err := a.withKey(func(key []byte) error {
		credentials, err := a.store.GetAllCredentials()
		if err != nil {
			return err
		}
		report.Total = len(credentials)

		// 摘要 -> 该密码对应的凭证列表
		byDigest := make(map[[32]byte][]model.AuditItem)
		// 摘要 -> 掩码，用于分组展示
		maskByDigest := make(map[[32]byte]string)

		for _, c := range credentials {
			plaintext, err := crypto.Decrypt(key, []byte(c.Secret))
			if err != nil {
				// 单条解密失败（如密文损坏）不应让整份报告失败，
				// 跳过该条继续，保证审计可用性。
				continue
			}

			secret := string(plaintext)
			// 用完立即清零解密缓冲
			for i := range plaintext {
				plaintext[i] = 0
			}

			item := model.AuditItem{
				ID:   c.ID,
				Name: c.Name,
				Type: int(c.Type),
			}

			// 1) 弱密码：仅对非空密码判定（空密码单独在下面标记）
			if secret == "" {
				item.Detail = "未设置密码"
				report.Weak = append(report.Weak, item)
			} else {
				strength := model.CheckPasswordStrength(secret)
				if strength == model.StrengthWeak {
					item.Detail = "强度不足"
					report.Weak = append(report.Weak, item)
				}
			}

			// 2) 重复密码：按摘要分组。空密码不参与重复检测
			//    （"都没设密码"不算重复使用风险，且会产生误导性分组）。
			if secret != "" {
				digest := sha256.Sum256([]byte(secret))
				byDigest[digest] = append(byDigest[digest], item)
				if _, ok := maskByDigest[digest]; !ok {
					maskByDigest[digest] = model.MaskSecret(secret)
				}
			}
		}

		// 汇总重复分组（只保留出现 2 次及以上的）
		for digest, items := range byDigest {
			if len(items) < 2 {
				continue
			}
			dupGroups = append(dupGroups, model.DuplicateGroup{
				Masked: maskByDigest[digest],
				Items:  items,
			})
		}
		return nil
	})
	if err != nil {
		return nil, err
	}

	// 稳定排序，避免每次打开面板顺序抖动
	sort.Slice(report.Weak, func(i, j int) bool { return report.Weak[i].Name < report.Weak[j].Name })
	sort.Slice(dupGroups, func(i, j int) bool {
		// 涉及凭证多的组排前面（风险更高）
		if len(dupGroups[i].Items) != len(dupGroups[j].Items) {
			return len(dupGroups[i].Items) > len(dupGroups[j].Items)
		}
		return dupGroups[i].Masked < dupGroups[j].Masked
	})

	// 组内也按名称排序
	for i := range dupGroups {
		items := dupGroups[i].Items
		sort.Slice(items, func(x, y int) bool { return items[x].Name < items[y].Name })
	}

	report.DuplicateGroups = dupGroups
	report.DuplicateGroupCount = len(dupGroups)

	dupItems := 0
	for _, g := range dupGroups {
		dupItems += len(g.Items)
	}
	report.DuplicateItemCount = dupItems
	report.WeakCount = len(report.Weak)

	report.Score = model.ComputeSecurityScore(report.Total, report.WeakCount, report.DuplicateItemCount)
	return report, nil
}
