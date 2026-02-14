#!/usr/bin/env bash
set -euo pipefail

# Netease Cloud Music API — Linux 一键部署脚本
# 用法:
#   ./deploy.sh install     安装并启动服务
#   ./deploy.sh uninstall   停止并卸载服务
#   ./deploy.sh status      查看服务状态
#   ./deploy.sh update      原地更新二进制（保留数据）

APP_NAME="netease-music-api"
INSTALL_DIR="/opt/${APP_NAME}"
SERVICE_NAME="${APP_NAME}.service"
RUN_USER="netease"
PORT=5000

# ─── 颜色 ────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
err()   { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# ─── 检测架构 ────────────────────────────────────────
detect_binary() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64)  BINARY_NAME="${APP_NAME}-linux-x64" ;;
        aarch64) BINARY_NAME="${APP_NAME}-linux-arm64" ;;
        *) err "不支持的架构: $arch (仅支持 x86_64 / aarch64)" ;;
    esac

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

    # 优先同目录，其次 dist/ 子目录
    if [[ -f "${SCRIPT_DIR}/${BINARY_NAME}" ]]; then
        BINARY_SRC="${SCRIPT_DIR}/${BINARY_NAME}"
    elif [[ -f "${SCRIPT_DIR}/dist/${BINARY_NAME}" ]]; then
        BINARY_SRC="${SCRIPT_DIR}/dist/${BINARY_NAME}"
    else
        err "找不到二进制文件: ${BINARY_NAME}\n  请将二进制放在脚本同目录或 dist/ 子目录下"
    fi
    info "检测到架构: $arch → $BINARY_NAME"
}

# ─── install ─────────────────────────────────────────
do_install() {
    [[ $EUID -ne 0 ]] && err "安装需要 root 权限，请使用 sudo"

    detect_binary

    info "创建部署目录: ${INSTALL_DIR}"
    mkdir -p "${INSTALL_DIR}"

    info "复制二进制（前端已嵌入）"
    cp "$BINARY_SRC" "${INSTALL_DIR}/${APP_NAME}"
    chmod +x "${INSTALL_DIR}/${APP_NAME}"

    # 创建专用用户（已存在则跳过）
    if ! id "$RUN_USER" &>/dev/null; then
        info "创建运行用户: ${RUN_USER}"
        useradd -r -s /usr/sbin/nologin "$RUN_USER"
    else
        info "用户 ${RUN_USER} 已存在，跳过创建"
    fi

    chown -R "${RUN_USER}:${RUN_USER}" "$INSTALL_DIR"

    info "写入 systemd 服务: ${SERVICE_NAME}"
    cat > "/etc/systemd/system/${SERVICE_NAME}" <<EOF
[Unit]
Description=Netease Cloud Music API
After=network.target

[Service]
Type=simple
User=${RUN_USER}
Group=${RUN_USER}
WorkingDirectory=${INSTALL_DIR}
ExecStart=${INSTALL_DIR}/${APP_NAME}
Restart=always
RestartSec=5

# 环境变量（按需修改）
Environment=HOST=0.0.0.0
Environment=PORT=${PORT}
Environment=LOG_LEVEL=info
Environment=DOWNLOADS_DIR=${INSTALL_DIR}/downloads
Environment=STATS_DIR=${INSTALL_DIR}/data
Environment=LOGS_DIR=${INSTALL_DIR}/logs
Environment=COOKIE_FILE=${INSTALL_DIR}/cookie.txt

# 安全加固
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${INSTALL_DIR}
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable "$SERVICE_NAME"
    systemctl start "$SERVICE_NAME"

    sleep 1

    if systemctl is-active --quiet "$SERVICE_NAME"; then
        ok "服务已启动"
        echo ""
        echo "  访问地址:  http://$(hostname -I | awk '{print $1}'):${PORT}"
        echo "  服务状态:  sudo systemctl status ${SERVICE_NAME}"
        echo "  查看日志:  sudo journalctl -u ${SERVICE_NAME} -f"
        echo "  部署目录:  ${INSTALL_DIR}"
        echo ""
        echo "  目录结构:"
        echo "    ${INSTALL_DIR}/"
        echo "    ├── ${APP_NAME}     # 单二进制（含前端）"
        echo "    ├── cookie.txt         # Cookie (自动创建)"
        echo "    ├── data/              # 统计数据 (自动创建)"
        echo "    ├── downloads/         # 下载缓存 (自动创建, 12h 清理)"
        echo "    └── logs/              # 日志文件 (自动创建)"
    else
        err "服务启动失败，请查看: journalctl -u ${SERVICE_NAME} -n 20"
    fi
}

# ─── uninstall ───────────────────────────────────────
do_uninstall() {
    [[ $EUID -ne 0 ]] && err "卸载需要 root 权限，请使用 sudo"

    info "停止并禁用服务"
    systemctl stop "$SERVICE_NAME" 2>/dev/null || true
    systemctl disable "$SERVICE_NAME" 2>/dev/null || true
    rm -f "/etc/systemd/system/${SERVICE_NAME}"
    systemctl daemon-reload

    echo ""
    read -rp "是否删除部署目录 ${INSTALL_DIR}？(包含 cookie/统计/日志) [y/N] " confirm
    if [[ "$confirm" =~ ^[Yy]$ ]]; then
        rm -rf "$INSTALL_DIR"
        ok "部署目录已删除"
    else
        info "保留部署目录: ${INSTALL_DIR}"
    fi

    read -rp "是否删除用户 ${RUN_USER}？[y/N] " confirm
    if [[ "$confirm" =~ ^[Yy]$ ]]; then
        userdel "$RUN_USER" 2>/dev/null || true
        ok "用户已删除"
    fi

    ok "卸载完成"
}

# ─── update ──────────────────────────────────────────
do_update() {
    [[ $EUID -ne 0 ]] && err "更新需要 root 权限，请使用 sudo"

    detect_binary

    info "停止服务"
    systemctl stop "$SERVICE_NAME"

    info "替换二进制"
    cp "$BINARY_SRC" "${INSTALL_DIR}/${APP_NAME}"
    chmod +x "${INSTALL_DIR}/${APP_NAME}"
    chown "${RUN_USER}:${RUN_USER}" "${INSTALL_DIR}/${APP_NAME}"

    info "重启服务"
    systemctl start "$SERVICE_NAME"

    sleep 1
    if systemctl is-active --quiet "$SERVICE_NAME"; then
        ok "更新完成，服务已重启"
    else
        err "服务启动失败，请查看: journalctl -u ${SERVICE_NAME} -n 20"
    fi
}

# ─── status ──────────────────────────────────────────
do_status() {
    echo ""
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        ok "服务运行中"
    else
        warn "服务未运行"
    fi
    echo ""
    systemctl status "$SERVICE_NAME" --no-pager 2>/dev/null || warn "服务未安装"
    echo ""

    if [[ -d "$INSTALL_DIR" ]]; then
        info "磁盘占用:"
        du -sh "${INSTALL_DIR}/downloads" 2>/dev/null || echo "  downloads/  (未创建)"
        du -sh "${INSTALL_DIR}/data" 2>/dev/null || echo "  data/       (未创建)"
        du -sh "${INSTALL_DIR}/logs" 2>/dev/null || echo "  logs/       (未创建)"
    fi
}

# ─── 入口 ────────────────────────────────────────────
case "${1:-}" in
    install)   do_install ;;
    uninstall) do_uninstall ;;
    update)    do_update ;;
    status)    do_status ;;
    *)
        echo "用法: $0 {install|uninstall|update|status}"
        echo ""
        echo "  install     安装并启动服务"
        echo "  uninstall   停止并卸载服务"
        echo "  update      原地更新二进制（保留数据）"
        echo "  status      查看服务状态"
        exit 1
        ;;
esac
