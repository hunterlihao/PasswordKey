import './style.css';

import { IsMasterSet, SetupMaster, Unlock, Lock, GetCredentials, GetCredentialViewById, GetFavorites, GetCredentialCounts, AddCredential, UpdateCredential, DeleteCredential, DeleteCredentials, ToggleFavorite, GetDecryptedSecret, GetDecryptedSecretDirect, GeneratePassword, CheckPasswordStrength, ChangeMasterPassword, ExportData, ExportSingle, ImportData, DownloadJSONTemplate, DownloadExcelTemplate, GetDbPath, ChangeDbPath, ResetDbPath } from '../wailsjs/go/main/App';

let currentView = 'login';
let currentFilter = -1;
let currentKeyword = '';
let searchTimer = null;
let credNotesMap = {};
let credCounts = {};
let selectedIds = new Set();
let selectMode = false;
let clipboardClearTimer = null;

// 常量
const SPLASH_ANIM_DELAY = 2600;
const SPLASH_UNLOCK_DELAY = 800;
const SPLASH_FADE_DELAY = 600;
const DEFAULT_PASSWORD_LENGTH = 16;
const CLIPBOARD_CLEAR_DELAY = 30000;
const SEARCH_DEBOUNCE_DELAY = 300;

const CRED_TYPES = {
    0: { icon: '🌐', badge: '浏览器', title: '浏览器密码', subtitle: '保存网站的账号和密码', editSubtitle: '修改网站登录信息',
         nameLabel: '名称', namePlaceholder: '例如: GitHub, Google',
         fields: [
             { id: 'url', label: '网址', placeholder: 'https://github.com', type: 'input' },
             { id: 'username', label: '用户名', placeholder: 'your@email.com', type: 'input' },
             { id: 'secret', label: '密码', placeholder: '输入密码', editPlaceholder: '留空则不修改', type: 'password', generate: true, strength: true },
         ], notesPlaceholder: '添加备注信息...' },
    1: { icon: '🔑', badge: 'API Key', title: 'API Key', subtitle: '保存服务密钥或访问令牌', editSubtitle: '修改密钥信息',
         nameLabel: '名称', namePlaceholder: '例如: OpenAI, AWS, Stripe',
         fields: [
             { id: 'secret', label: 'API Key', placeholder: '粘贴您的 API Key', editPlaceholder: '留空则不修改', type: 'textarea', rows: 4 },
         ], notesPlaceholder: '添加备注信息，例如用途、有效期等...' },
    2: { icon: '🔐', badge: 'SSH', title: 'SSH 密钥', subtitle: '保存 SSH 私钥和连接信息', editSubtitle: '修改 SSH 连接信息',
         nameLabel: '连接名称', namePlaceholder: '例如: 生产服务器, GitHub SSH',
         fields: [
             { id: 'url', label: '主机地址', placeholder: '192.168.1.100 或 github.com', type: 'input' },
             { id: 'username', label: '用户名', placeholder: 'root, ubuntu, git', type: 'input' },
             { id: 'secret', label: '私钥', placeholder: '粘贴私钥内容', editPlaceholder: '留空则不修改', type: 'textarea', rows: 8 },
         ], notesPlaceholder: '端口、认证方式、用途等...' },
    3: { icon: '📝', badge: '笔记', title: '安全笔记', subtitle: '保存加密的敏感文本信息', editSubtitle: '修改笔记内容',
         nameLabel: '标题', namePlaceholder: '例如: WiFi密码, 保险柜密码',
         fields: [
             { id: 'secret', label: '笔记内容', placeholder: '输入需要加密保存的文本内容', editPlaceholder: '留空则不修改', type: 'textarea', rows: 8 },
         ], notesPlaceholder: '添加备注信息...' },
    4: { icon: '🗄️', badge: '数据库', title: '数据库连接', subtitle: '保存数据库连接凭据', editSubtitle: '修改数据库连接信息',
         nameLabel: '连接名称', namePlaceholder: '例如: MySQL 生产库, Redis 缓存',
         fields: [
             { id: 'url', label: '连接地址', placeholder: 'localhost:3306 或 db.example.com:5432', type: 'input' },
             { id: 'username', label: '用户名', placeholder: 'admin, root', type: 'input' },
             { id: 'secret', label: '密码', placeholder: '输入密码', editPlaceholder: '留空则不修改', type: 'password' },
         ], notesPlaceholder: '数据库类型、端口、用途等...' },
    5: { icon: '📧', badge: '邮箱', title: '邮箱账号', subtitle: '保存邮箱登录信息', editSubtitle: '修改邮箱登录信息',
         nameLabel: '邮箱名称', namePlaceholder: '例如: Gmail, 企业邮箱',
         fields: [
             { id: 'url', label: 'SMTP 服务器', placeholder: 'smtp.gmail.com', type: 'input' },
             { id: 'username', label: '邮箱地址', placeholder: 'user@example.com', type: 'input' },
             { id: 'secret', label: '密码/授权码', placeholder: '输入密码或授权码', editPlaceholder: '留空则不修改', type: 'password' },
         ], notesPlaceholder: 'IMAP/POP3 服务器、端口等...' },
    6: { icon: '💳', badge: '银行卡', title: '银行卡/信用卡', subtitle: '保存银行卡信息', editSubtitle: '修改银行卡信息',
         nameLabel: '银行名称', namePlaceholder: '例如: 招商银行, 工商银行信用卡',
         fields: [
             { id: 'username', label: '卡号', placeholder: '6225 **** **** 1234', type: 'input' },
             { id: 'secret', label: 'CVV / 查询密码', placeholder: '输入 CVV 或查询密码', editPlaceholder: '留空则不修改', type: 'password' },
         ], notesPlaceholder: '过期日期、持卡人、开户行等...' },
};

function getCredIcon(type) { return (CRED_TYPES[type] || CRED_TYPES[0]).icon; }
function getCredBadge(type) { return (CRED_TYPES[type] || CRED_TYPES[0]).badge; }

async function init() {
    const splash = document.getElementById('splash');

    // 立即发 IsMasterSet，和 splash 动画并行，
    // 动画结束时 RPC 也回来了，不额外等待
    const masterPromise = IsMasterSet().catch(() => null);

    if (splash) {
        // 等动画播放完毕
        await new Promise(r => setTimeout(r, SPLASH_ANIM_DELAY));
        // 锁开启动画
        const lockSvg = splash.querySelector('.lock-svg');
        if (lockSvg) lockSvg.classList.add('unlocked');
        await new Promise(r => setTimeout(r, SPLASH_UNLOCK_DELAY));
        // 淡出
        splash.classList.add('fade-out');
        await new Promise(r => setTimeout(r, SPLASH_FADE_DELAY));
        splash.remove();
    }

    try {
        const isMasterSet = await masterPromise;
        if (isMasterSet) {
            showLoginView();
        } else {
            showSetupView();
        }
    } catch (err) {
        document.querySelector('#app').innerHTML = `
            <div style="text-align:center;padding:60px 20px;">
                <h2>应用初始化失败</h2>
                <p style="margin-top:12px;color:#888;">请重启应用，如问题持续请联系开发者。</p>
            </div>`;
    }
}

function showSetupView() {
    currentView = 'setup';
    document.querySelector('#app').innerHTML = `
        <div class="auth-container">
            <div class="auth-card">
                <div class="logo">
                    <div class="icon">🔐</div>
                    <h1>密码管理器</h1>
                    <p class="subtitle">安全存储您的密码和密钥</p>
                </div>
                <div class="form-group">
                    <label>主密钥</label>
                    <input type="password" id="password" placeholder="设置您的主密钥" autocomplete="off">
                    <div class="strength-indicator">
                        <div class="strength-bar"><div class="strength-fill" id="strength-fill"></div></div>
                        <span class="strength-text" id="strength-text"></span>
                    </div>
                </div>
                <div class="form-group">
                    <label>确认密钥</label>
                    <input type="password" id="confirm" placeholder="再次输入主密钥" autocomplete="off">
                </div>
                <div class="buttons">
                    <button class="btn btn-primary" onclick="setupMaster()">开始使用</button>
                </div>
            </div>
        </div>
    `;

    document.getElementById('password').addEventListener('input', (e) => {
        updateStrength(e.target.value);
    });
}

function showLoginView() {
    currentView = 'login';
    document.querySelector('#app').innerHTML = `
        <div class="auth-container">
            <div class="auth-card">
                <div class="logo">
                    <div class="icon">🔐</div>
                    <h1>欢迎回来</h1>
                    <p class="subtitle">输入主密钥解锁您的密码库</p>
                </div>
                <div class="form-group">
                    <label>主密钥</label>
                    <input type="password" id="password" placeholder="输入主密钥" autocomplete="off">
                </div>
                <div class="buttons">
                    <button class="btn btn-primary" onclick="unlock()">解锁</button>
                </div>
            </div>
        </div>
    `;

    document.getElementById('password').addEventListener('keydown', (e) => {
        if (e.key === 'Enter') unlock();
    });
}

window.showMainView = async function() {
    currentView = 'main';
    credCounts = await GetCredentialCounts();
    const c = (t) => credCounts[t] != null ? ` <span class="tab-count">${credCounts[t]}</span>` : '';
    document.querySelector('#app').innerHTML = `
        <div class="app-container">
            <div class="header">
                <div class="header-left">
                    <div class="header-logo">🔐</div>
                    <span class="header-title">密码管理器</span>
                </div>
                <div class="search-box">
                    <input type="text" id="search" placeholder="搜索凭证..." oninput="searchCredentials(this.value)">
                </div>
                <div class="header-actions">
                    <button class="btn btn-primary btn-small" onclick="showTypeSelection()">+ 新增</button>
                    <button class="btn btn-ghost btn-small" id="batch-delete-btn" onclick="toggleSelectMode()">批量删除</button>
                    <button class="btn btn-ghost btn-small" onclick="exportData()">导出</button>
                    <button class="btn btn-ghost btn-small" onclick="importData()">导入</button>
                    <button class="btn btn-ghost btn-small" onclick="showTemplateMenu()">模板</button>
                    <button class="btn btn-ghost btn-small" onclick="showChangePasswordForm()">改密</button>
                    <button class="btn btn-ghost btn-small" onclick="showSettings()">设置</button>
                    <button class="btn btn-ghost btn-small" onclick="lockApp()">锁屏</button>
                </div>
            </div>
            <div class="tabs">
                <button class="tab ${currentFilter === -1 ? 'active' : ''}" data-type="-1" onclick="filterCredentials(-1)">全部${c(-1)}</button>
                <button class="tab ${currentFilter === 0 ? 'active' : ''}" data-type="0" onclick="filterCredentials(0)">🌐 浏览器${c(0)}</button>
                <button class="tab ${currentFilter === 1 ? 'active' : ''}" data-type="1" onclick="filterCredentials(1)">🔑 API Key${c(1)}</button>
                <button class="tab ${currentFilter === 2 ? 'active' : ''}" data-type="2" onclick="filterCredentials(2)">🔐 SSH${c(2)}</button>
                <button class="tab ${currentFilter === 3 ? 'active' : ''}" data-type="3" onclick="filterCredentials(3)">📝 笔记${c(3)}</button>
                <button class="tab ${currentFilter === 4 ? 'active' : ''}" data-type="4" onclick="filterCredentials(4)">🗄️ 数据库${c(4)}</button>
                <button class="tab ${currentFilter === 5 ? 'active' : ''}" data-type="5" onclick="filterCredentials(5)">📧 邮箱${c(5)}</button>
                <button class="tab ${currentFilter === 6 ? 'active' : ''}" data-type="6" onclick="filterCredentials(6)">💳 银行卡${c(6)}</button>
                <button class="tab ${currentFilter === -2 ? 'active' : ''}" data-type="-2" onclick="filterCredentials(-2)">★ 收藏${c(-2)}</button>
            </div>
            <div class="credential-list" id="credential-list">
                <div class="empty-state">
                    <div class="icon">⏳</div>
                    <h3>加载中...</h3>
                </div>
            </div>
            <div class="selection-bar" id="selection-bar" style="display:none">
                <span id="selection-count">已选 0 项</span>
                <button class="btn btn-danger btn-small" onclick="batchDelete()">删除选中</button>
                <button class="btn btn-ghost btn-small" onclick="clearSelection()">取消</button>
            </div>
        </div>
    `;
    loadCredentials();
    // 同步批量删除按钮状态
    const batchBtn = document.getElementById('batch-delete-btn');
    if (batchBtn) {
        batchBtn.classList.toggle('active', selectMode);
        batchBtn.textContent = selectMode ? '取消选择' : '批量删除';
    }
    updateSelectionBar();
}

async function refreshCounts() {
    try {
        credCounts = await GetCredentialCounts();
        renderTabCounts();
    } catch (_) {}
}

// 根据 credCounts 渲染各 tab 的数量徽标
function renderTabCounts() {
    document.querySelectorAll('.tab[data-type]').forEach(tab => {
        const t = parseInt(tab.dataset.type);
        const count = credCounts[t] || 0;
        const existing = tab.querySelector('.tab-count');
        if (existing) existing.textContent = count;
        else if (count > 0) tab.insertAdjacentHTML('beforeend', ` <span class="tab-count">${count}</span>`);
    });
}

async function loadCredentials() {
    renderTabCounts();
    try {
        const credentials = currentFilter === -2
            ? await GetFavorites(currentKeyword)
            : await GetCredentials(currentFilter, currentKeyword);
        const listEl = document.getElementById('credential-list');

        if (!credentials || credentials.length === 0) {
            listEl.innerHTML = `
                <div class="empty-state">
                    <div class="icon">📭</div>
                    <h3>暂无凭证</h3>
                    <p>点击"新增"按钮添加您的第一个密码</p>
                </div>
            `;
            return;
        }

        credNotesMap = {};
        credentials.forEach(c => { if (c.notes) credNotesMap[c.id] = c.notes; });

        listEl.innerHTML = credentials.map((c, i) => `
            <div class="credential-item${selectedIds.has(c.id) ? ' selected' : ''}" data-id="${c.id}" style="--i:${i}" onclick="handleCredClick(${c.id}, event)">
                <div class="credential-icon">
                    ${getCredIcon(c.type)}
                </div>
                <div class="credential-info">
                    <div class="credential-header">
                        <span class="credential-name">${escapeHtml(c.name)}</span>
                        <span class="credential-badge">${getCredBadge(c.type)}</span>
                    </div>
                    <div class="credential-meta">
                        ${c.url ? `<span>${escapeHtml(c.url)}</span>` : ''}
                        ${c.username ? `<span>${escapeHtml(c.username)}</span>` : ''}
                    </div>
                </div>
                <div class="credential-actions" onclick="event.stopPropagation()">
                    <button class="favorite-btn ${c.is_favorite ? 'active' : 'inactive'}" onclick="toggleFavorite(${c.id})">
                        ${c.is_favorite ? '★' : '☆'}
                    </button>
                    <button class="btn btn-ghost btn-small" onclick="editCredential(${c.id})">编辑</button>
                    <button class="btn btn-ghost btn-small" onclick="exportSingle(${c.id})">导出</button>
                    <button class="btn btn-ghost btn-small" onclick="deleteCredential(${c.id})" style="color: var(--danger)">删除</button>
                </div>
            </div>
        `).join('');
    } catch (err) {
        const listEl = document.getElementById('credential-list');
        if (listEl) {
            listEl.innerHTML = `
                <div class="empty-state">
                    <div class="icon">⚠️</div>
                    <h3>加载失败</h3>
                    <p>${escapeHtml(String(err))}</p>
                </div>
            `;
        }
    }
}

// ===== Tooltip =====

let tooltipEl = null;

function ensureTooltip() {
    if (tooltipEl) return;
    tooltipEl = document.createElement('div');
    tooltipEl.className = 'cred-tooltip';
    tooltipEl.innerHTML = '<div class="cred-tooltip-content"></div>';
    document.body.appendChild(tooltipEl);
}

function showTooltip(item, notes) {
    ensureTooltip();
    tooltipEl.querySelector('.cred-tooltip-content').textContent = notes;

    // 先显示以获取尺寸
    tooltipEl.style.left = '-9999px';
    tooltipEl.style.top = '-9999px';
    tooltipEl.classList.add('visible');

    const rect = item.getBoundingClientRect();
    const ttRect = tooltipEl.getBoundingClientRect();
    const viewportH = window.innerHeight;

    // 默认显示在上方，如果上方空间不足则显示在下方
    const spaceAbove = rect.top;
    const spaceBelow = viewportH - rect.bottom;
    const showBelow = spaceAbove < ttRect.height + 14 && spaceBelow > ttRect.height + 14;

    let top;
    if (showBelow) {
        top = rect.bottom + 10;
        tooltipEl.className = 'cred-tooltip visible arrow-up';
    } else {
        top = rect.top - ttRect.height - 10;
        tooltipEl.className = 'cred-tooltip visible arrow-down';
    }

    // 水平居中，不超出视口
    let left = rect.left + rect.width / 2 - ttRect.width / 2;
    left = Math.max(12, Math.min(left, window.innerWidth - ttRect.width - 12));

    tooltipEl.style.left = left + 'px';
    tooltipEl.style.top = top + 'px';
}

function hideTooltip() {
    if (tooltipEl) {
        tooltipEl.classList.remove('visible');
    }
}

// 使用事件委托，避免为每个 item 绑定事件
document.addEventListener('mouseover', (e) => {
    const item = e.target.closest('.credential-item');
    if (!item) return;
    const id = item.dataset.id;
    const notes = id && credNotesMap[id];
    if (notes) showTooltip(item, notes);
});

document.addEventListener('mouseout', (e) => {
    const item = e.target.closest('.credential-item');
    if (!item) { hideTooltip(); return; }
    const related = e.relatedTarget;
    if (!related || !item.contains(related)) hideTooltip();
});

// 列表滚动时隐藏 tooltip
document.addEventListener('scroll', hideTooltip, true);

// ===== 新增：选择类型页面 =====

window.showTypeSelection = function() {
    currentView = 'type-select';
    document.querySelector('#app').innerHTML = `
        <div class="type-selection">
            <div class="type-card">
                <h2>新增凭证</h2>
                <p class="form-subtitle">选择要添加的凭证类型</p>
                <div class="type-options">
                    <div class="type-option" onclick="showBrowserForm()">
                        <div class="type-icon">🌐</div>
                        <h3>浏览器密码</h3>
                        <p>保存网站登录账号和密码</p>
                    </div>
                    <div class="type-option" onclick="showApiKeyForm()">
                        <div class="type-icon">🔑</div>
                        <h3>API Key</h3>
                        <p>保存服务密钥和访问令牌</p>
                    </div>
                    <div class="type-option" onclick="showSSHKeyForm()">
                        <div class="type-icon">🔐</div>
                        <h3>SSH 密钥</h3>
                        <p>保存 SSH 私钥和连接信息</p>
                    </div>
                    <div class="type-option" onclick="showSecureNoteForm()">
                        <div class="type-icon">📝</div>
                        <h3>安全笔记</h3>
                        <p>保存加密的敏感文本信息</p>
                    </div>
                    <div class="type-option" onclick="showDatabaseForm()">
                        <div class="type-icon">🗄️</div>
                        <h3>数据库连接</h3>
                        <p>保存数据库连接凭据</p>
                    </div>
                    <div class="type-option" onclick="showEmailForm()">
                        <div class="type-icon">📧</div>
                        <h3>邮箱账号</h3>
                        <p>保存邮箱登录信息</p>
                    </div>
                    <div class="type-option" onclick="showBankCardForm()">
                        <div class="type-icon">💳</div>
                        <h3>银行卡/信用卡</h3>
                        <p>保存银行卡信息</p>
                    </div>
                </div>
                <div class="type-back">
                    <button class="btn btn-ghost" onclick="showMainView()">← 返回列表</button>
                </div>
            </div>
        </div>
    `;
};

// ===== 通用表单渲染 =====

function showCredForm(typeId, cred) {
    const isEdit = cred !== undefined && cred !== null;
    const cfg = CRED_TYPES[typeId] || CRED_TYPES[0];
    currentView = `form-${typeId}`;

    const fieldsHtml = cfg.fields.map(f => {
        const val = isEdit && f.id !== 'secret' ? escapeHtml(cred[f.id] || '') : '';
        const ph = isEdit && f.editPlaceholder ? f.editPlaceholder : f.placeholder;

        if (f.type === 'textarea') {
            const style = f.id === 'secret' ? ` style="font-family: 'JetBrains Mono', monospace; font-size: 13px; line-height: 1.6;"` : '';
            return `<div class="form-group"><label>${escapeHtml(f.label)}</label><textarea id="${f.id}" rows="${f.rows || 4}" placeholder="${escapeHtml(ph)}"${style}>${val}</textarea></div>`;
        }
        if (f.type === 'password') {
            let extra = '';
            if (f.generate) extra = `<button class="btn btn-secondary btn-small" onclick="generatePassword()">生成</button>`;
            let strength = '';
            if (f.strength) strength = `<div class="strength-indicator"><div class="strength-bar"><div class="strength-fill" id="strength-fill"></div></div><span class="strength-text" id="strength-text"></span></div>`;
            return `<div class="form-group"><label>${escapeHtml(f.label)}</label><div class="password-input"><input type="password" id="${f.id}" value="" placeholder="${escapeHtml(ph)}"><button class="btn btn-icon" onclick="toggleSecretVisibility()" title="显示/隐藏">👁</button>${extra}</div>${strength}</div>`;
        }
        return `<div class="form-group"><label>${escapeHtml(f.label)}</label><input type="text" id="${f.id}" value="${val}" placeholder="${escapeHtml(ph)}"></div>`;
    }).join('\n                ');

    const nameVal = isEdit ? escapeHtml(cred.name) : '';

    document.querySelector('#app').innerHTML = `
        <div class="form-page">
            <div class="form-card">
                <h2>${isEdit ? '编辑' + cfg.title : '新增' + cfg.title}</h2>
                <p class="form-subtitle">${isEdit ? cfg.editSubtitle : cfg.subtitle}</p>
                <input type="hidden" id="type" value="${typeId}">

                <div class="form-group">
                    <label>${escapeHtml(cfg.nameLabel)}</label>
                    <input type="text" id="name" value="${nameVal}" placeholder="${escapeHtml(cfg.namePlaceholder)}">
                </div>

                ${fieldsHtml}

                <div class="form-group">
                    <label>备注</label>
                    <textarea id="notes" placeholder="${escapeHtml(cfg.notesPlaceholder)}">${isEdit ? escapeHtml(cred.notes) : ''}</textarea>
                </div>

                <div class="buttons">
                    <button class="btn btn-secondary" onclick="showMainView()">取消</button>
                    <button class="btn btn-primary" onclick="saveCredential(${isEdit ? cred.id : 0})">保存</button>
                </div>
            </div>
        </div>
    `;

    // 密码强度监听
    const secretEl = document.getElementById('secret');
    if (secretEl && cfg.fields.some(f => f.strength)) {
        secretEl.addEventListener('input', (e) => updateStrength(e.target.value));
    }
}

// 保持向后兼容的别名
window.showBrowserForm = (cred) => showCredForm(0, cred);
window.showApiKeyForm = (cred) => showCredForm(1, cred);
window.showSSHKeyForm = (cred) => showCredForm(2, cred);
window.showSecureNoteForm = (cred) => showCredForm(3, cred);
window.showDatabaseForm = (cred) => showCredForm(4, cred);
window.showEmailForm = (cred) => showCredForm(5, cred);
window.showBankCardForm = (cred) => showCredForm(6, cred);

// ===== 编辑入口 =====

window.editCredential = async function(id) {
    try {
        const cred = await GetCredentialViewById(id);
        if (!cred) return;
        showCredForm(cred.type, cred);
    } catch (err) {
        showToast('加载失败: ' + err, 'error');
    }
};

// ===== 通用保存 =====

window.saveCredential = async function(id) {
    const type = parseInt(document.getElementById('type').value);
    const name = document.getElementById('name').value;
    const url = document.getElementById('url')?.value || '';
    const username = document.getElementById('username')?.value || '';
    const secret = document.getElementById('secret').value;
    const notes = document.getElementById('notes').value;

    if (!name) {
        showToast('名称不能为空', 'error');
        return;
    }

    if (!secret && id === 0) {
        const cfg = CRED_TYPES[type];
        const secretLabel = cfg ? cfg.fields.find(f => f.id === 'secret').label : '密码';
        showToast(secretLabel + '不能为空', 'error');
        return;
    }

    const cred = { id, name, type, url, username, secret, notes, is_favorite: 0 };

    try {
        if (id > 0) {
            await UpdateCredential(cred);
        } else {
            await AddCredential(cred);
        }
        showToast('保存成功', 'success');
        showMainView();
    } catch (err) {
        showToast('保存失败: ' + err, 'error');
    }
};

// ===== 查看密码（二次验证） =====

window.viewCredential = async function(id) {
    let cred;
    try {
        cred = await GetCredentialViewById(id);
    } catch (err) {
        showToast('加载失败: ' + err, 'error');
        return;
    }
    if (!cred) return;

    let secret = '';
    try {
        secret = await GetDecryptedSecretDirect(id);
    } catch (err) {
        showToast('解密失败: ' + err, 'error');
        return;
    }

    const cfg = CRED_TYPES[cred.type] || CRED_TYPES[0];
    const monoTypes = [1, 2, 3];
    const fields = [
        { label: cfg.nameLabel, value: cred.name, mono: false },
    ];
    cfg.fields.forEach(f => {
        if (f.id === 'secret') {
            fields.push({ label: f.label, value: secret, mono: monoTypes.includes(cred.type) });
        } else if (f.id !== 'secret' && cred[f.id]) {
            fields.push({ label: f.label, value: cred[f.id], mono: false });
        }
    });
    if (cred.notes) {
        fields.push({ label: '备注', value: cred.notes, mono: false });
    }

    const fieldsHTML = fields.map(f => `
            <div class="form-group">
                <label>${escapeHtml(f.label)}</label>
                <div class="field-copy-row">
                    <div class="password-display" style="${f.mono ? 'font-family: \'JetBrains Mono\', monospace; font-size: 13px; white-space: pre-wrap; word-break: break-all;' : ''}">${escapeHtml(f.value)}</div>
                    <button class="btn-copy" onclick="copyToClipboard(this.previousElementSibling.textContent)" title="复制">📋</button>
                </div>
            </div>
    `).join('');

    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal modal-wide">
            <h3>${escapeHtml(cred.name)}</h3>
            ${fieldsHTML}
            <div class="modal-buttons">
                <button class="btn btn-primary" onclick="this.closest('.modal-overlay').remove()">关闭</button>
            </div>
        </div>
    `;
    document.body.appendChild(modal);
    modal.addEventListener('click', (e) => { if (e.target === modal) modal.remove(); });
};

// ===== 删除（需要主密钥验证） =====

window.deleteCredential = async function(id) {
    showVerifyModal('删除凭证', async (password) => {
        try {
            await GetDecryptedSecret(id, password);
            closeAllModals();

            const confirmModal = document.createElement('div');
            confirmModal.className = 'modal-overlay';
            confirmModal.innerHTML = `
                <div class="modal">
                    <h3>确认删除</h3>
                    <p class="text-sm text-muted">身份已验证，确定要删除此凭证吗？此操作不可撤销。</p>
                    <div class="modal-buttons">
                        <button class="btn btn-secondary" onclick="this.closest('.modal-overlay').remove()">取消</button>
                        <button class="btn btn-danger" id="confirm-delete">删除</button>
                    </div>
                </div>
            `;
            document.body.appendChild(confirmModal);
            confirmModal.addEventListener('click', (e) => { if (e.target === confirmModal) confirmModal.remove(); });

            confirmModal.querySelector('#confirm-delete').onclick = async () => {
                try {
                    await DeleteCredential(id);
                    confirmModal.remove();
                    showToast('已删除', 'success');
                    refreshCounts();
                    loadCredentials();
                } catch (err) {
                    showToast('删除失败: ' + err, 'error');
                }
            };
        } catch (err) {
            showToast('验证失败，无法删除: ' + err, 'error');
        }
    });
};

// ===== 批量选择 =====

window.toggleSelectMode = function() {
    selectMode = !selectMode;
    const btn = document.getElementById('batch-delete-btn');
    if (btn) {
        btn.classList.toggle('active', selectMode);
        btn.textContent = selectMode ? '取消选择' : '批量删除';
    }
    if (!selectMode) clearSelection();
};

window.handleCredClick = function(id, event) {
    if (!selectMode) {
        viewCredential(id);
        return;
    }
    event.stopPropagation();
    if (selectedIds.has(id)) {
        selectedIds.delete(id);
    } else {
        selectedIds.add(id);
    }
    const item = document.querySelector(`.credential-item[data-id="${id}"]`);
    if (item) item.classList.toggle('selected', selectedIds.has(id));
    updateSelectionBar();
};

function updateSelectionBar() {
    const bar = document.getElementById('selection-bar');
    const count = document.getElementById('selection-count');
    if (!bar) return;
    if (selectedIds.size > 0) {
        bar.style.display = 'flex';
        count.textContent = `已选 ${selectedIds.size} 项`;
    } else {
        bar.style.display = 'none';
    }
}

window.clearSelection = function() {
    selectedIds.clear();
    document.querySelectorAll('.credential-item.selected').forEach(el => el.classList.remove('selected'));
    updateSelectionBar();
};

window.batchDelete = function() {
    if (selectedIds.size === 0) return;
    const count = selectedIds.size;

    showVerifyModal('批量删除', async (password) => {
        try {
            const ids = Array.from(selectedIds);
            await GetDecryptedSecret(ids[0], password);
            closeAllModals();

            const confirmModal = document.createElement('div');
            confirmModal.className = 'modal-overlay';
            confirmModal.innerHTML = `
                <div class="modal">
                    <h3>确认批量删除</h3>
                    <p class="text-sm text-muted">身份已验证，确定要删除选中的 ${count} 条凭证吗？此操作不可撤销。</p>
                    <div class="modal-buttons">
                        <button class="btn btn-secondary" onclick="this.closest('.modal-overlay').remove()">取消</button>
                        <button class="btn btn-danger" id="confirm-batch-delete">删除 ${count} 条</button>
                    </div>
                </div>
            `;
            document.body.appendChild(confirmModal);
            confirmModal.addEventListener('click', (e) => { if (e.target === confirmModal) confirmModal.remove(); });

            confirmModal.querySelector('#confirm-batch-delete').onclick = async () => {
                try {
                    await DeleteCredentials(ids);
                    confirmModal.remove();
                    clearSelection();
                    selectMode = false;
                    const btn = document.getElementById('batch-delete-btn');
                    if (btn) { btn.classList.remove('active'); btn.textContent = '批量删除'; }
                    showToast(`已删除 ${count} 条凭证`, 'success');
                    refreshCounts();
                    loadCredentials();
                } catch (err) {
                    showToast('批量删除失败: ' + err, 'error');
                }
            };
        } catch (err) {
            showToast('验证失败: ' + err, 'error');
        }
    });
};

// ===== 通用验证弹窗 =====

function showVerifyModal(title, onSuccess) {
    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal">
            <h3>验证身份</h3>
            <p class="text-sm text-muted">请输入主密钥以${title}</p>
            <div class="form-group mt-4">
                <input type="password" id="verify-password" placeholder="输入主密钥">
            </div>
            <div class="modal-buttons">
                <button class="btn btn-secondary" onclick="this.closest('.modal-overlay').remove()">取消</button>
                <button class="btn btn-primary" id="verify-btn">确认</button>
            </div>
        </div>
    `;
    document.body.appendChild(modal);

    const input = modal.querySelector('#verify-password');
    input.focus();

    modal.querySelector('#verify-btn').onclick = () => {
        const password = input.value;
        if (!password) return;
        onSuccess(password);
    };

    input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') modal.querySelector('#verify-btn').click();
    });

    modal.addEventListener('click', (e) => {
        if (e.target === modal) modal.remove();
    });
}

function closeAllModals() {
    document.querySelectorAll('.modal-overlay').forEach(m => m.remove());
}

function showConfirmModal(title, message) {
    return new Promise((resolve) => {
        const modal = document.createElement('div');
        modal.className = 'modal-overlay';
        modal.innerHTML = `
            <div class="modal">
                <h3>${escapeHtml(title)}</h3>
                <p class="text-sm text-muted">${escapeHtml(message)}</p>
                <div class="modal-buttons">
                    <button class="btn btn-secondary" id="confirm-cancel">取消</button>
                    <button class="btn btn-primary" id="confirm-ok">确认</button>
                </div>
            </div>
        `;
        document.body.appendChild(modal);
        modal.addEventListener('click', (e) => { if (e.target === modal) { modal.remove(); resolve(false); } });
        modal.querySelector('#confirm-cancel').onclick = () => { modal.remove(); resolve(false); };
        modal.querySelector('#confirm-ok').onclick = () => { modal.remove(); resolve(true); };
    });
}

// ===== 工具函数 =====

window.toggleSecretVisibility = function() {
    const input = document.getElementById('secret');
    if (input) input.type = input.type === 'password' ? 'text' : 'password';
};

window.generatePassword = async function() {
    const password = await GeneratePassword(DEFAULT_PASSWORD_LENGTH);
    document.getElementById('secret').value = password;
    document.getElementById('secret').type = 'text';
    updateStrength(password);
};

function updateStrength(password) {
    const fill = document.getElementById('strength-fill');
    const text = document.getElementById('strength-text');
    if (!fill || !text) return;

    if (!password) {
        fill.style.width = '0';
        text.textContent = '';
        return;
    }

    CheckPasswordStrength(password).then(strength => {
        const levels = ['弱', '中', '强'];
        const colors = ['#dc2626', '#d97706', '#16a34a'];
        const widths = ['33%', '66%', '100%'];
        const idx = Math.max(0, Math.min(2, strength || 0));

        fill.style.width = widths[idx];
        fill.style.background = colors[idx];
        fill.style.boxShadow = `0 0 8px ${colors[idx]}40`;
        text.textContent = `密码强度: ${levels[idx]}`;
        text.style.color = colors[idx];
    });
}

// 写剪贴板统一入口：优先用 Async Clipboard API（需要安全上下文），
// WebView2 通过 with_html(NavigateToString) 加载时不是安全上下文，
// navigator.clipboard 为 undefined，降级到 execCommand('copy')（同步，
// 依赖用户手势，点击按钮即满足）。
// 返回 Promise，成功 resolve，失败 reject。
function writeClipboardText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
        return navigator.clipboard.writeText(text);
    }
    return new Promise((resolve, reject) => {
        const textarea = document.createElement('textarea');
        textarea.value = text;
        // 移出可视区域，避免页面跳动
        textarea.style.position = 'fixed';
        textarea.style.opacity = '0';
        document.body.appendChild(textarea);
        textarea.focus();
        textarea.select();
        let ok = false;
        try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
        document.body.removeChild(textarea);
        if (ok) { resolve(); } else { reject(new Error('execCommand copy 失败')); }
    });
}

window.copyToClipboard = function(text) {
    writeClipboardText(text).then(() => {
        showToast('已复制到剪贴板，30秒后自动清除', 'success');
        scheduleClipboardClear(text);
    }).catch(() => {
        showToast('复制失败', 'error');
    });
};

// 30 秒后清空剪贴板（若内容仍是本次复制的值）。
// execCommand 降级模式下无法读取剪贴板做比较，跳过。
function scheduleClipboardClear(text) {
    if (clipboardClearTimer) clearTimeout(clipboardClearTimer);
    clipboardClearTimer = setTimeout(() => {
        if (navigator.clipboard && navigator.clipboard.readText) {
            navigator.clipboard.readText().then(current => {
                if (current === text) {
                    navigator.clipboard.writeText('').catch(() => {});
                }
            }).catch(() => {});
        }
        clipboardClearTimer = null;
    }, CLIPBOARD_CLEAR_DELAY);
}

window.toggleFavorite = async function(id) {
    try {
        await ToggleFavorite(id);

        // 局部更新：切换按钮样式和图标
        const item = document.querySelector(`.credential-item[data-id="${id}"]`);
        if (item) {
            const btn = item.querySelector('.favorite-btn');
            const isActive = btn.classList.contains('active');
            btn.classList.toggle('active', !isActive);
            btn.classList.toggle('inactive', isActive);
            btn.innerHTML = isActive ? '☆' : '★';
        }

        // 刷新各 tab 的计数徽标（收藏数变化）
        refreshCounts();

        // 收藏筛选下取消收藏时，需要移除该项
        if (currentFilter === -2) {
            loadCredentials();
        }
    } catch (err) {
        showToast('操作失败: ' + err, 'error');
    }
};

window.filterCredentials = function(type) {
    currentFilter = type;
    clearSelection();
    document.querySelectorAll('.tab').forEach(tab => {
        tab.classList.toggle('active', parseInt(tab.dataset.type) === type);
    });
    loadCredentials();
};

window.searchCredentials = function(keyword) {
    currentKeyword = keyword;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => loadCredentials(), SEARCH_DEBOUNCE_DELAY);
};

window.setupMaster = async function() {
    const password = document.getElementById('password').value;
    const confirm = document.getElementById('confirm').value;

    if (!password) {
        showToast('密码不能为空', 'error');
        return;
    }

    if (password !== confirm) {
        showToast('两次输入的密码不一致', 'error');
        return;
    }

    try {
        await SetupMaster(password);
        showToast('设置成功', 'success');
        showMainView();
    } catch (err) {
        showToast('设置失败: ' + err, 'error');
    }
};

window.unlock = async function() {
    const password = document.getElementById('password').value;

    if (!password) {
        showToast('密码不能为空', 'error');
        return;
    }

    try {
        await Unlock(password);
        showMainView();
    } catch (err) {
        showToast('' + err, 'error');
    }
};

window.lockApp = async function() {
    try {
        await Lock();
    } catch (err) {
        // 锁定操作即使失败也应返回登录页
    }
    showLoginView();
};

// ===== 修改主密钥 =====

window.showChangePasswordForm = function() {
    currentView = 'change-password';
    document.querySelector('#app').innerHTML = `
        <div class="form-page">
            <div class="form-card" style="max-width: 460px;">
                <h2>修改主密钥</h2>
                <p class="form-subtitle">输入旧密码和新密码来更换主密钥</p>

                <div class="form-group">
                    <label>当前主密钥</label>
                    <input type="password" id="old-password" placeholder="输入当前主密钥" autocomplete="off">
                </div>

                <div class="form-group">
                    <label>新主密钥</label>
                    <input type="password" id="new-password" placeholder="设置新的主密钥" autocomplete="off">
                    <div class="strength-indicator">
                        <div class="strength-bar"><div class="strength-fill" id="strength-fill"></div></div>
                        <span class="strength-text" id="strength-text"></span>
                    </div>
                </div>

                <div class="form-group">
                    <label>确认新密钥</label>
                    <input type="password" id="confirm-password" placeholder="再次输入新主密钥" autocomplete="off">
                </div>

                <div class="buttons">
                    <button class="btn btn-secondary" onclick="showMainView()">取消</button>
                    <button class="btn btn-primary" onclick="changePassword()">确认修改</button>
                </div>
            </div>
        </div>
    `;

    document.getElementById('new-password').addEventListener('input', (e) => {
        updateStrength(e.target.value);
    });
};

window.changePassword = async function() {
    const oldPassword = document.getElementById('old-password').value;
    const newPassword = document.getElementById('new-password').value;
    const confirmPassword = document.getElementById('confirm-password').value;

    if (!oldPassword || !newPassword) {
        showToast('密码不能为空', 'error');
        return;
    }

    if (newPassword !== confirmPassword) {
        showToast('两次输入的新密码不一致', 'error');
        return;
    }

    if (oldPassword === newPassword) {
        showToast('新密码不能与旧密码相同', 'error');
        return;
    }

    try {
        await ChangeMasterPassword(oldPassword, newPassword);
        showToast('主密钥修改成功', 'success');
        showMainView();
    } catch (err) {
        showToast('修改失败: ' + err, 'error');
    }
};

// ===== 导入导出 =====

window.exportSingle = function(id) {
    showExportFormatModal(async (format) => {
        try {
            const path = await ExportSingle(id, format);
            showToast('已导出: ' + path, 'success');
        } catch (err) {
            if (!String(err).includes('已取消')) {
                showToast('导出失败: ' + err, 'error');
            }
        }
    });
};

window.exportData = async function() {
    const counts = await GetCredentialCounts();
    const total = Object.values(counts).reduce((s, v) => s + v, 0);
    if (total === 0) {
        showToast('没有凭证可导出', 'error');
        return;
    }
    showExportFormatModal(async (format) => {
        const confirmed = await showConfirmModal(
            '安全提醒',
            '导出的文件包含明文密码，请妥善保管并及时删除不再需要的文件。是否继续导出？'
        );
        if (!confirmed) return;
        try {
            const path = await ExportData(format);
            showToast('已导出到: ' + path, 'success');
        } catch (err) {
            if (!String(err).includes('已取消')) {
                showToast('导出失败: ' + err, 'error');
            }
        }
    });
};

function showExportFormatModal(onSelect) {
    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal">
            <h3>选择导出格式</h3>
            <p class="text-sm text-muted">请选择导出文件的格式</p>
            <div class="modal-buttons" style="flex-direction: column; gap: 10px; margin-top: 20px;">
                <button class="btn btn-secondary" id="export-json" style="width: 100%">JSON 格式</button>
                <button class="btn btn-secondary" id="export-excel" style="width: 100%">Excel 格式</button>
                <button class="btn btn-ghost" onclick="this.closest('.modal-overlay').remove()" style="width: 100%">取消</button>
            </div>
        </div>
    `;
    document.body.appendChild(modal);
    modal.addEventListener('click', (e) => { if (e.target === modal) modal.remove(); });
    modal.querySelector('#export-json').onclick = () => { modal.remove(); onSelect('json'); };
    modal.querySelector('#export-excel').onclick = () => { modal.remove(); onSelect('excel'); };
}

window.importData = async function() {
    try {
        const count = await ImportData();
        showToast(`成功导入 ${count} 条凭证`, 'success');
        refreshCounts();
        loadCredentials();
    } catch (err) {
        if (!String(err).includes('已取消')) {
            showToast('导入失败: ' + err, 'error');
        }
    }
};

window.showTemplateMenu = function() {
    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal">
            <h3>下载模板</h3>
            <p class="text-sm text-muted">选择模板格式，用于批量导入凭证</p>
            <div class="modal-buttons" style="flex-direction: column; gap: 10px; margin-top: 20px;">
                <button class="btn btn-secondary" onclick="downloadJSONTemplate()" style="width: 100%">JSON 模板</button>
                <button class="btn btn-secondary" onclick="downloadExcelTemplate()" style="width: 100%">Excel 模板</button>
                <button class="btn btn-ghost" onclick="this.closest('.modal-overlay').remove()" style="width: 100%">取消</button>
            </div>
        </div>
    `;
    document.body.appendChild(modal);
    modal.addEventListener('click', (e) => { if (e.target === modal) modal.remove(); });
};

window.downloadJSONTemplate = async function() {
    closeAllModals();
    try {
        const path = await DownloadJSONTemplate();
        showToast('模板已保存: ' + path, 'success');
    } catch (err) {
        if (!String(err).includes('已取消')) {
            showToast('下载失败: ' + err, 'error');
        }
    }
};

window.downloadExcelTemplate = async function() {
    closeAllModals();
    try {
        const path = await DownloadExcelTemplate();
        showToast('模板已保存: ' + path, 'success');
    } catch (err) {
        if (!String(err).includes('已取消')) {
            showToast('下载失败: ' + err, 'error');
        }
    }
};

function escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

function showToast(message, type = 'success') {
    const toast = document.createElement('div');
    toast.className = `toast ${type}`;
    toast.textContent = message;
    document.body.appendChild(toast);

    setTimeout(() => {
        toast.style.animation = 'toastIn 0.3s ease-out reverse';
        setTimeout(() => toast.remove(), 300);
    }, 3000);
}

window.showSettings = async function() {
    let dbPath = '';
    try {
        dbPath = await GetDbPath();
    } catch (_) {}

    const defaultDir = dbPath.replace(/[\\\/][^\\\/]+$/, '');
    const isCustom = !dbPath.includes('AppData\\Roaming\\PasswordManager') && !dbPath.includes('AppData/Roaming/PasswordManager');

    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal settings-modal">
            <h3>设置</h3>
            <div class="settings-section">
                <label class="settings-label">数据库存储位置</label>
                <div class="settings-path-row">
                    <code class="settings-path" id="db-path-display">${escapeHtml(dbPath)}</code>
                    <button class="btn btn-ghost btn-small" onclick="copyPath()">复制</button>
                </div>
                <div class="settings-actions">
                    <button class="btn btn-primary btn-small" onclick="changeDbPath()">更改路径</button>
                    ${isCustom ? '<button class="btn btn-secondary btn-small" onclick="resetDbPath()">恢复默认</button>' : ''}
                </div>
                <p class="settings-hint">更改后需重启应用生效</p>
            </div>
            <div class="modal-buttons">
                <button class="btn btn-ghost" onclick="this.closest('.modal-overlay').remove()">关闭</button>
            </div>
        </div>
    `;
    document.body.appendChild(modal);
    modal.addEventListener('click', (e) => { if (e.target === modal) modal.remove(); });
};

window.copyPath = function() {
    const pathEl = document.getElementById('db-path-display');
    if (pathEl) {
        writeClipboardText(pathEl.textContent).then(() => {
            showToast('已复制到剪贴板', 'success');
        }).catch(() => {
            showToast('复制失败', 'error');
        });
    }
};

window.changeDbPath = async function() {
    try {
        const newPath = await ChangeDbPath();
        closeAllModals();
        showToast('数据库路径已更改: ' + newPath + '\n请重启应用', 'success');
    } catch (err) {
        if (!String(err).includes('已取消')) {
            showToast('更改失败: ' + err, 'error');
        }
    }
};

window.resetDbPath = async function() {
    try {
        await ResetDbPath();
        closeAllModals();
        showToast('已恢复默认路径，请重启应用', 'success');
    } catch (err) {
        showToast('恢复失败: ' + err, 'error');
    }
};

init();
