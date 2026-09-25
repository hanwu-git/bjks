// 检查 Tauri API 是否可用
let invoke;
try {
    invoke = window.__TAURI__.core.invoke;
} catch (e) {
    console.error('Tauri API 加载失败:', e);
    document.body.innerHTML = '<div style="padding:20px;color:red;font-size:16px;">错误：无法加载 Tauri API。请确保使用最新版 WebView2。<br>' + e + '</div>';
    throw e;
}

// 状态管理
let sortOrders = [{ field: 'Name', asc: true }];
let searchTimeout = null;
let selectedIndex = -1;
let currentResults = [];

// DOM 元素
const searchInput = document.getElementById('searchInput');
const extInput = document.getElementById('extInput');
const extTypeSelect = document.getElementById('extTypeSelect');
const settingsBtn = document.getElementById('settingsBtn');
const aboutBtn = document.getElementById('aboutBtn');
const indexStatus = document.getElementById('indexStatus');
const searchStatus = document.getElementById('searchStatus');
const resultsBody = document.getElementById('resultsBody');
const paginationInfo = document.getElementById('paginationInfo');
const contextMenu = document.getElementById('contextMenu');
const settingsPanel = document.getElementById('settingsPanel');
const closeSettings = document.getElementById('closeSettings');
const saveSettings = document.getElementById('saveSettings');
const rescanBtn = document.getElementById('rescanBtn');
const aboutModal = document.getElementById('aboutModal');
const closeAbout = document.getElementById('closeAbout');
const versionText = document.getElementById('versionText');
const readmeBtn = document.getElementById('readmeBtn');
const readmeModal = document.getElementById('readmeModal');
const closeReadme = document.getElementById('closeReadme');
const readmeText = document.getElementById('readmeText');

// 文件类型到扩展名的映射
const TYPE_EXTENSIONS = {
    audio: 'mp3 wav flac aac ogg wma m4a',
    compressed: 'zip rar 7z tar gz bz2 xz',
    document: 'doc docx pdf txt xls xlsx ppt pptx md csv',
    executable: 'exe msi bat cmd ps1',
    image: 'jpg jpeg png gif bmp webp svg ico',
    video: 'mp4 avi mkv mov wmv flv webm'
};

// 字段显示名称
const FIELD_LABELS = {
    Name: '名称',
    Size: '大小',
    Modified: '修改时间'
};

// 系统风格 SVG 文件图标（参考 Windows 资源管理器）
const FILE_ICONS = {
    folder: '<svg viewBox="0 0 16 16" width="16" height="16"><path fill="#F4D03F" d="M1.5 3h4l1.5 2h7a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-12a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/><path fill="#D4AC0D" d="M1.5 3h4l1.5 2h7a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-12a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" fill="none" stroke="#B7950B" stroke-width="0.6"/></svg>',
    doc: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#2B579A"/><path fill="#fff" d="M4 4h8v1H4zm0 2h8v1H4zm0 2h5v1H4z"/><text x="8" y="13" font-size="4" fill="#fff" text-anchor="middle" font-family="Arial" font-weight="bold">W</text></svg>',
    xls: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#217346"/><path fill="#fff" d="M4 4h8v1H4zm0 2h8v1H4zm0 2h5v1H4z"/><text x="8" y="13" font-size="4" fill="#fff" text-anchor="middle" font-family="Arial" font-weight="bold">X</text></svg>',
    ppt: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#D24726"/><path fill="#fff" d="M4 4h8v1H4zm0 2h8v1H4zm0 2h5v1H4z"/><text x="8" y="13" font-size="4" fill="#fff" text-anchor="middle" font-family="Arial" font-weight="bold">P</text></svg>',
    pdf: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#E74C3C"/><path fill="#fff" d="M4 4h8v1H4zm0 2h8v1H4z"/><text x="8" y="13" font-size="4" fill="#fff" text-anchor="middle" font-family="Arial" font-weight="bold">PDF</text></svg>',
    txt: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#F5F5F5" stroke="#BDBDBD" stroke-width="0.6"/><path stroke="#9E9E9E" stroke-width="0.7" d="M4 5h8M4 7.5h8M4 10h5"/></svg>',
    image: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#E8F5E9" stroke="#81C784" stroke-width="0.6"/><circle cx="6" cy="6" r="1.5" fill="#66BB6A"/><path fill="none" stroke="#66BB6A" stroke-width="1" d="M3 12l3-3 2 2 3-3 2 2"/></svg>',
    audio: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#E3F2FD" stroke="#64B5F6" stroke-width="0.6"/><path fill="#2196F3" d="M6 4v7h2V9.5a2 2 0 1 1 0-3V4H6z"/></svg>',
    video: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#F3E5F5" stroke="#BA68C8" stroke-width="0.6"/><rect x="4" y="5" width="8" height="5" rx="0.5" fill="#9C27B0"/><path fill="#fff" d="M7 6.5l3 1.5-3 1.5z"/></svg>',
    archive: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#FFF3E0" stroke="#FFB74D" stroke-width="0.6"/><rect x="6" y="3" width="4" height="2" fill="#FF9800"/><path stroke="#FF9800" stroke-width="1" d="M8 5v8"/></svg>',
    exe: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1.5" fill="#E3F2FD" stroke="#1976D2" stroke-width="0.7"/><g transform="translate(1, 0)"><circle cx="6.5" cy="6.5" r="3.2" fill="none" stroke="#0D47A1" stroke-width="1.8"/><path fill="none" stroke="#0D47A1" stroke-width="1.8" stroke-linecap="round" d="M9 9l3 3"/></g></svg>',
    default: '<svg viewBox="0 0 16 16" width="16" height="16"><rect x="2" y="1" width="12" height="14" rx="1" fill="#FAFAFA" stroke="#BDBDBD" stroke-width="0.6"/><path stroke="#9E9E9E" stroke-width="0.7" d="M4 5h6M4 7.5h5M4 10h4"/></svg>'
};

function getFileIcon(entry) {
    if (entry.is_dir) return FILE_ICONS.folder;
    const ext = entry.extension.toLowerCase();
    if (['doc', 'docx'].includes(ext)) return FILE_ICONS.doc;
    if (['xls', 'xlsx', 'csv'].includes(ext)) return FILE_ICONS.xls;
    if (['ppt', 'pptx'].includes(ext)) return FILE_ICONS.ppt;
    if (ext === 'pdf') return FILE_ICONS.pdf;
    if (['txt', 'md', 'log', 'ini', 'json', 'xml', 'yml', 'yaml', 'conf'].includes(ext)) return FILE_ICONS.txt;
    if (['jpg', 'jpeg', 'png', 'gif', 'bmp', 'webp', 'svg', 'ico', 'tiff'].includes(ext)) return FILE_ICONS.image;
    if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'wma', 'm4a'].includes(ext)) return FILE_ICONS.audio;
    if (['mp4', 'avi', 'mkv', 'mov', 'wmv', 'flv', 'webm'].includes(ext)) return FILE_ICONS.video;
    if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz'].includes(ext)) return FILE_ICONS.archive;
    if (['exe', 'msi', 'bat', 'cmd', 'ps1'].includes(ext)) return FILE_ICONS.exe;
    return FILE_ICONS.default;
}

// 初始化
document.addEventListener('DOMContentLoaded', async () => {
    setupEventListeners();
    updateSortButtons();
    try {
        await updateIndexStatus();
    } catch (e) {
        console.error('初始化更新索引状态失败:', e);
    }
    // 每 3 秒刷新一次索引状态（扫描过程中实时更新）
    setInterval(updateIndexStatus, 3000);
});

// 设置事件监听
function setupEventListeners() {
    // 搜索输入（防抖 300ms）
    searchInput.addEventListener('input', () => {
        clearTimeout(searchTimeout);
        searchTimeout = setTimeout(() => {
            performSearch();
        }, 300);
    });

    // 扩展名输入（防抖 300ms）
    extInput.addEventListener('input', () => {
        clearTimeout(searchTimeout);
        searchTimeout = setTimeout(() => {
            performSearch();
        }, 300);
    });

    // 文件类型选择
    extTypeSelect.addEventListener('change', () => {
        const type = extTypeSelect.value;
        if (type === 'all' || type === 'folder') {
            extInput.value = '';
        } else {
            extInput.value = TYPE_EXTENSIONS[type] || '';
        }
        performSearch();
    });

    // 键盘导航
    searchInput.addEventListener('keydown', handleKeyboard);

    // 设置按钮
    settingsBtn.addEventListener('click', openSettings);
    closeSettings.addEventListener('click', closeSettingsPanel);
    saveSettings.addEventListener('click', saveSettingsHandler);
    rescanBtn.addEventListener('click', rescanHandler);

    // 关于弹窗
    aboutBtn.addEventListener('click', openAbout);
    closeAbout.addEventListener('click', closeAboutModal);
    readmeBtn.addEventListener('click', openReadme);
    closeReadme.addEventListener('click', closeReadmeModal);
    aboutModal.addEventListener('click', (e) => { if (e.target === aboutModal) closeAboutModal(); });
    readmeModal.addEventListener('click', (e) => { if (e.target === readmeModal) closeReadmeModal(); });

    // 排序按钮
    document.querySelectorAll('.sort-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const field = e.target.dataset.sort;
            const isCtrl = e.ctrlKey || e.metaKey;
            handleSortClick(field, isCtrl);
        });
    });

    // 右键菜单
    resultsBody.addEventListener('contextmenu', showContextMenu);
    document.addEventListener('click', hideContextMenu);
    contextMenu.querySelectorAll('.menu-item').forEach(item => {
        item.addEventListener('click', handleMenuAction);
    });

    // 添加屏蔽规则
    document.getElementById('addExcludePath').addEventListener('click', () => addExcludeItem('path'));
    document.getElementById('addExcludePattern').addEventListener('click', () => addExcludeItem('pattern'));
    document.getElementById('addExcludeExtension').addEventListener('click', () => addExcludeItem('extension'));
}

// 处理排序点击
function handleSortClick(field, isCtrl) {
    const existingIndex = sortOrders.findIndex(o => o.field === field);

    if (isCtrl) {
        // 组合排序：Ctrl+点击添加/切换/移除字段
        if (existingIndex >= 0) {
            sortOrders[existingIndex].asc = !sortOrders[existingIndex].asc;
        } else {
            sortOrders.push({ field, asc: true });
        }
    } else {
        // 单字段排序：普通点击重置为单个字段
        if (existingIndex >= 0 && sortOrders.length === 1) {
            sortOrders[0].asc = !sortOrders[0].asc;
        } else {
            sortOrders = [{ field, asc: true }];
        }
    }

    updateSortButtons();
    performSearch();
}

// 键盘导航
function handleKeyboard(e) {
    if (e.key === 'ArrowDown') {
        e.preventDefault();
        selectedIndex = Math.min(selectedIndex + 1, currentResults.length - 1);
        updateSelection();
    } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        selectedIndex = Math.max(selectedIndex - 1, 0);
        updateSelection();
    } else if (e.key === 'Enter' && selectedIndex >= 0) {
        e.preventDefault();
        openFolder(currentResults[selectedIndex].path);
    }
}

// 更新选择状态
function updateSelection() {
    const rows = resultsBody.querySelectorAll('tr');
    rows.forEach((row, idx) => {
        if (idx === selectedIndex) {
            row.classList.add('selected');
            row.scrollIntoView({ block: 'nearest' });
        } else {
            row.classList.remove('selected');
        }
    });
}

// 执行搜索
async function performSearch() {
    try {
        const sortOrdersForBackend = sortOrders.map(o => ({
            field: o.field,
            asc: o.asc
        }));

        const request = {
            query: searchInput.value.trim(),
            ext_query: extInput.value.trim(),
            only_folders: extTypeSelect.value === 'folder',
            sort_by: sortOrders[0]?.field || 'Name',
            sort_asc: sortOrders[0]?.asc ?? true,
            sort_orders: sortOrdersForBackend,
            offset: 0,
            limit: 1000
        };

        const response = await invoke('search', { request });
        currentResults = response.results;

        renderResults(response);
        searchStatus.textContent = `搜索耗时 ${response.query_time_ms}ms`;
    } catch (error) {
        console.error('搜索失败:', error);
    }
}

// 渲染结果
function renderResults(response) {
    resultsBody.innerHTML = '';

    response.results.forEach((entry, index) => {
        const tr = document.createElement('tr');
        tr.dataset.index = index;

        const icon = getFileIcon(entry);
        const size = entry.is_dir ? '' : formatSize(entry.size);
        const modified = entry.modified ? formatDateTime(entry.modified) : '';

        tr.innerHTML = `
            <td class="col-icon">${icon}</td>
            <td class="col-name"><span class="file-name">${escapeHtml(entry.name)}</span></td>
            <td class="col-path">${escapeHtml(entry.parent_path)}</td>
            <td class="col-modified">${modified}</td>
            <td class="col-size">${size}</td>
        `;

        tr.addEventListener('click', () => {
            selectedIndex = index;
            updateSelection();
        });

        resultsBody.appendChild(tr);
    });

    paginationInfo.textContent = `共 ${response.total} 条结果 | 第 1/1 页`;
    selectedIndex = -1;
}

// 更新索引状态
async function updateIndexStatus() {
    try {
        const status = await invoke('get_index_status');
        if (status.is_scanning) {
            indexStatus.textContent = `正在建立索引… 已扫描 ${status.scanned_files.toLocaleString()} 个文件`;
            indexStatus.style.color = '#0078d4';
        } else {
            indexStatus.textContent = `已索引 ${status.scanned_files.toLocaleString()} 个文件`;
            indexStatus.style.color = '';
        }
    } catch (error) {
        console.error('获取索引状态失败:', error);
        indexStatus.textContent = `索引状态获取失败: ${error}`;
        indexStatus.style.color = '#e74c3c';
    }
}

// 更新排序按钮显示
function updateSortButtons() {
    document.querySelectorAll('.sort-btn').forEach(btn => {
        const field = btn.dataset.sort;
        const orderIndex = sortOrders.findIndex(o => o.field === field);

        btn.classList.toggle('active', orderIndex >= 0);

        if (orderIndex >= 0) {
            const arrow = sortOrders[orderIndex].asc ? '▼' : '▲';
            const label = FIELD_LABELS[field] || field;
            const orderNum = sortOrders.length > 1 ? `${orderIndex + 1}` : '';
            btn.textContent = `${label}${orderNum} ${arrow}`;
        } else {
            const label = FIELD_LABELS[field] || field;
            btn.textContent = label;
        }
    });
}

// 打开关于弹窗
async function openAbout() {
    try {
        const info = await invoke('get_app_version');
        versionText.textContent = `${info.name} v${info.version}`;
    } catch (e) {
        versionText.textContent = '未知版本';
        console.error('获取版本失败:', e);
    }
    aboutModal.style.display = 'flex';
}

function closeAboutModal() {
    aboutModal.style.display = 'none';
}

// 打开说明文档弹窗
async function openReadme() {
    try {
        const text = await invoke('get_readme');
        readmeText.textContent = text;
    } catch (e) {
        readmeText.textContent = '无法加载说明文档: ' + e;
        console.error('获取说明文档失败:', e);
    }
    closeAboutModal();
    readmeModal.style.display = 'flex';
}

function closeReadmeModal() {
    readmeModal.style.display = 'none';
}

// 显示右键菜单
function showContextMenu(e) {
    e.preventDefault();
    const target = e.target.closest('tr');
    if (!target) return;

    selectedIndex = parseInt(target.dataset.index);
    updateSelection();

    contextMenu.style.display = 'block';
    contextMenu.style.left = e.clientX + 'px';
    contextMenu.style.top = e.clientY + 'px';
}

// 隐藏右键菜单
function hideContextMenu() {
    contextMenu.style.display = 'none';
}

// 处理菜单操作
async function handleMenuAction(e) {
    const action = e.target.dataset.action;
    if (selectedIndex < 0) return;

    const entry = currentResults[selectedIndex];

    switch (action) {
        case 'open':
            await invoke('open_file', { path: entry.path });
            break;
        case 'openFolder':
            await openFolder(entry.path);
            break;
        case 'copyPath':
            await navigator.clipboard.writeText(entry.path);
            break;
    }

    hideContextMenu();
}

// 打开文件所在目录
async function openFolder(path) {
    try {
        await invoke('open_folder', { path });
    } catch (error) {
        console.error('打开目录失败:', error);
    }
}

// 打开设置面板
async function openSettings() {
    settingsPanel.style.display = 'flex';
    await loadSettings();
}

// 关闭设置面板
function closeSettingsPanel() {
    settingsPanel.style.display = 'none';
}

// 加载设置
async function loadSettings() {
    try {
        const config = await invoke('get_config');

        renderExcludeList('excludePathsList', config.excluded_paths, 'path');
        renderExcludeList('excludePatternsList', config.excluded_file_patterns, 'pattern');
        renderExcludeList('excludeExtensionsList', config.excluded_extensions, 'extension');
    } catch (error) {
        console.error('加载设置失败:', error);
    }
}

// 渲染屏蔽列表
function renderExcludeList(elementId, items, type) {
    const list = document.getElementById(elementId);
    list.innerHTML = '';

    items.forEach((item, index) => {
        const li = document.createElement('li');
        li.innerHTML = `
            <span>${escapeHtml(item)}</span>
            <button onclick="removeExcludeItem('${type}', ${index})">删除</button>
        `;
        list.appendChild(li);
    });
}

// 添加屏蔽项
async function addExcludeItem(type) {
    const prompts = {
        path: '请输入要屏蔽的目录路径（如 C:\\Windows）:',
        pattern: '请输入要屏蔽的文件模式（如 *.tmp）:',
        extension: '请输入要屏蔽的扩展名（如 log）:'
    };

    const value = prompt(prompts[type]);
    if (!value) return;

    try {
        if (type === 'path') {
            await invoke('add_exclude_path', { path: value });
        }
        await loadSettings();
    } catch (error) {
        console.error('添加屏蔽项失败:', error);
    }
}

// 删除屏蔽项
window.removeExcludeItem = async function(type, index) {
    try {
        const config = await invoke('get_config');

        if (type === 'path') {
            const path = config.excluded_paths[index];
            await invoke('remove_exclude_path', { path });
        }

        await loadSettings();
    } catch (error) {
        console.error('删除屏蔽项失败:', error);
    }
};

// 保存设置
async function saveSettingsHandler() {
    try {
        const config = await invoke('get_config');
        await invoke('save_settings', { config });
        alert('设置已保存');
        closeSettingsPanel();
    } catch (error) {
        console.error('保存设置失败:', error);
        alert('保存设置失败');
    }
}

// 重新扫描
async function rescanHandler() {
    try {
        await invoke('rescan');
        alert('重新扫描已启动');
        await updateIndexStatus();
    } catch (error) {
        console.error('重新扫描失败:', error);
    }
}

// 格式化文件大小
function formatSize(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
}

// 格式化日期时间（Unix 时间戳，秒）
function formatDateTime(timestamp) {
    if (!timestamp) return '';
    const date = new Date(timestamp * 1000);
    const y = date.getFullYear();
    const m = String(date.getMonth() + 1).padStart(2, '0');
    const d = String(date.getDate()).padStart(2, '0');
    const h = String(date.getHours()).padStart(2, '0');
    const min = String(date.getMinutes()).padStart(2, '0');
    return `${y}-${m}-${d} ${h}:${min}`;
}

// HTML 转义
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}
