package model

// AuditItem 审计报告中的单条问题条目。
// 只包含展示所需的最小字段，不含密码明文。
type AuditItem struct {
	ID   int64  `json:"id"`
	Name string `json:"name"`
	Type int    `json:"type"`
	// Detail 补充说明，例如弱密码的强度级别。
	Detail string `json:"detail"`
}

// DuplicateGroup 一组使用相同密码的凭证。
type DuplicateGroup struct {
	// Masked 密码的掩码形式（仅用于让用户分辨是哪一组，不泄露原文）。
	Masked string `json:"masked"`
	// Items 使用该密码的全部凭证。
	Items []AuditItem `json:"items"`
}

// AuditReport 安全体检报告。
type AuditReport struct {
	// Total 凭证总数。
	Total int `json:"total"`
	// WeakCount 弱密码数量。
	WeakCount int `json:"weak_count"`
	// DuplicateGroupCount 存在重复的密码组数。
	DuplicateGroupCount int `json:"duplicate_group_count"`
	// DuplicateItemCount 涉及重复密码的凭证总数。
	DuplicateItemCount int `json:"duplicate_item_count"`
	// Weak 弱密码列表。
	Weak []AuditItem `json:"weak"`
	// DuplicateGroups 重复密码分组。
	DuplicateGroups []DuplicateGroup `json:"duplicate_groups"`
	// Score 安全评分（0-100）。
	Score int `json:"score"`
}

// MaskSecret 把密码转成掩码形式用于展示，只保留首尾各 1 位，
// 中间统一用 * 替代并把长度归一化，避免通过长度猜测密码。
// 空串返回固定占位符。
func MaskSecret(secret string) string {
	if secret == "" {
		return "(空密码)"
	}
	r := []rune(secret)
	if len(r) == 1 {
		return "*"
	}
	// 固定输出形如 "a***z"，长度不随原密码变化
	return string(r[0]) + "***" + string(r[len(r)-1])
}

// ComputeSecurityScore 根据问题数量估算 0-100 的安全评分。
//
// 计分思路：从满分开始按比例扣分，弱密码与重复密码各占一半权重。
// 没有凭证时返回 100（无可指摘）。
func ComputeSecurityScore(total, weakCount, duplicateItemCount int) int {
	if total <= 0 {
		return 100
	}

	// 各自折算成「问题凭证占比」，累加后按 100 分制扣减。
	// 用两个指标各自的占比而非绝对数量，避免凭证多时被过度惩罚。
	weakRatio := float64(weakCount) / float64(total)
	dupRatio := float64(duplicateItemCount) / float64(total)

	penalty := (weakRatio*0.6 + dupRatio*0.4) * 100
	score := int(100 - penalty)
	if score < 0 {
		score = 0
	}
	if score > 100 {
		score = 100
	}
	return score
}
