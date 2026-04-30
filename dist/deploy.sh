#!/usr/bin/env bash
set -euo pipefail

# Netease Cloud Music API — Linux 一键部署脚本
# 用法:
#   ./deploy.sh              一键安装并启动（已安装则覆盖更新）
#   ./deploy.sh install      同上
#   ./deploy.sh uninstall    停止并卸载服务（默认保留数据）
#   ./deploy.sh update       原地更新二进制（保留数据，hash 无变化则跳过重启）
#   ./deploy.sh status       查看服务状态

APP_NAME="netease-music-api"
INSTALL_DIR="/opt/${APP_NAME}"
SERVICE_NAME="${APP_NAME}.service"
RUN_USER="netease"
PORT=5000

# ─── 颜色 ────────────────────────────────────────────
if [[ -t 1 ]]; then
    RED=$'\033[0;31m'; GREEN=$'\033[0;32m'; YELLOW=$'\033[1;33m'; CYAN=$'\033[0;36m'; NC=$'\033[0m'
else
    RED=''; GREEN=''; YELLOW=''; CYAN=''; NC=''
fi
info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
err()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

# ─── 自动提权（普通用户 → sudo 自重启）─────────────
escalate_if_needed() {
    if [[ $EUID -ne 0 ]]; then
        if command -v sudo >/dev/null 2>&1; then
            info "需要 root 权限，自动 sudo 重启..."
            exec sudo -E bash "$0" "$@"
        else
            err "需要 root 权限，且系统未安装 sudo。请用 root 用户运行。"
        fi
    fi
}

# ─── 环境前置检查 ───────────────────────────────────
preflight_check() {
    command -v systemctl >/dev/null 2>&1 \
        || err "需要 systemd（systemctl 未找到）。本脚本不支持 OpenRC / SysVinit / Alpine 默认 init。"
}

# ─── 检测架构 ────────────────────────────────────────
detect_binary() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)   BINARY_NAME="${APP_NAME}-linux-x64" ;;
        aarch64|arm64)  BINARY_NAME="${APP_NAME}-linux-arm64" ;;
        *) err "不支持的架构: $arch (仅支持 x86_64 / aarch64)" ;;
    esac

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

    # 优先同目录，其次 dist/ 子目录
    if [[ -f "${SCRIPT_DIR}/${BINARY_NAME}" ]]; then
        BINARY_SRC="${SCRIPT_DIR}/${BINARY_NAME}"
    elif [[ -f "${SCRIPT_DIR}/dist/${BINARY_NAME}" ]]; then
        BINARY_SRC="${SCRIPT_DIR}/dist/${BINARY_NAME}"
    else
        err "找不到二进制文件: ${BINARY_NAME}
  期望位置:
    ${SCRIPT_DIR}/${BINARY_NAME}
    ${SCRIPT_DIR}/dist/${BINARY_NAME}
  请将二进制 cp 到脚本同目录后重试。"
    fi
    info "架构: $arch → $(basename "$BINARY_SRC") ($(du -h "$BINARY_SRC" | awk '{print $1}'))"
}

# ─── 端口占用检测（非阻断，仅警告）─────────────────
check_port_free() {
    local in_use=""
    if command -v ss >/dev/null 2>&1; then
        in_use=$(ss -tlnH 2>/dev/null | awk '{print $4}' | grep -E ":(${PORT})$" || true)
    elif command -v netstat >/dev/null 2>&1; then
        in_use=$(netstat -tln 2>/dev/null | awk '{print $4}' | grep -E ":(${PORT})$" || true)
    fi
    if [[ -n "$in_use" ]]; then
        warn "端口 ${PORT} 已被占用 ($in_use)；服务启动可能失败，必要时改 systemd unit 中 PORT=。"
    fi
}

# ─── 防火墙提示（非自动放行）───────────────────────
firewall_hint() {
    if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q "Status: active"; then
        warn "检测到 ufw 已启用，远程访问需手动放行: sudo ufw allow ${PORT}/tcp"
    fi
    if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
        warn "检测到 firewalld 已启用，远程访问需手动放行:"
        warn "  sudo firewall-cmd --add-port=${PORT}/tcp --permanent && sudo firewall-cmd --reload"
    fi
}

# ─── 健康检查（curl /health）────────────────────────
health_check() {
    if ! command -v curl >/dev/null 2>&1; then
        info "未安装 curl，跳过 /health 自检（systemd 已上报 active 即视为成功）"
        return 0
    fi
    local i
    for i in 1 2 3 4 5; do
        if curl -fs --max-time 3 "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
            ok "/health 通过 (尝试 $i 次)"
            return 0
        fi
        sleep 1
    done
    warn "/health 未通过 — 查日志: journalctl -u ${SERVICE_NAME} -n 30"
}

# ─── 写 systemd unit ─────────────────────────────────
write_systemd_unit() {
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

# 资源限制（针对网易云 CDN 高频连接）
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
}

# ─── install ─────────────────────────────────────────
do_install() {
    detect_binary
    check_port_free

    # 已存在则改走 update 流程（保留数据 / 用户）
    if [[ -f "/etc/systemd/system/${SERVICE_NAME}" ]]; then
        info "检测到已安装 → 切换 update 路径（保留 cookie / 数据 / 日志）"
        do_update
        return 0
    fi

    info "创建部署目录: ${INSTALL_DIR}"
    mkdir -p "${INSTALL_DIR}"

    info "复制二进制（前端已嵌入）"
    install -m 0755 "$BINARY_SRC" "${INSTALL_DIR}/${APP_NAME}"

    # 创建专用用户（已存在则跳过）
    if ! id "$RUN_USER" &>/dev/null; then
        info "创建运行用户: ${RUN_USER}"
        useradd -r -s /usr/sbin/nologin -d "${INSTALL_DIR}" "$RUN_USER"
    else
        info "用户 ${RUN_USER} 已存在，跳过创建"
    fi

    chown -R "${RUN_USER}:${RUN_USER}" "$INSTALL_DIR"

    write_systemd_unit
    systemctl daemon-reload
    systemctl enable "$SERVICE_NAME" >/dev/null 2>&1
    systemctl start "$SERVICE_NAME"

    sleep 1

    if systemctl is-active --quiet "$SERVICE_NAME"; then
        ok "服务已启动"
        firewall_hint
        health_check

        local lan_ip
        lan_ip=$(hostname -I 2>/dev/null | awk '{print $1}')
        echo ""
        echo "  访问地址:  http://${lan_ip:-<本机 IP>}:${PORT}"
        echo "  服务状态:  sudo systemctl status ${SERVICE_NAME}"
        echo "  查看日志:  sudo journalctl -u ${SERVICE_NAME} -f"
        echo "  部署目录:  ${INSTALL_DIR}"
        echo ""
        echo "  目录结构:"
        echo "    ${INSTALL_DIR}/"
        echo "    ├── ${APP_NAME}     # 单二进制（含前端）"
        echo "    ├── cookie.txt         # Cookie (首次访问 UI 引导)"
        echo "    ├── data/              # 统计数据 / admin.hash / runtime_config.json"
        echo "    ├── downloads/         # 下载缓存 (12h 自动清理)"
        echo "    └── logs/              # 日志文件"
    else
        echo ""
        warn "服务未进入 active 状态，最近 20 行日志:"
        journalctl -u "$SERVICE_NAME" -n 20 --no-pager 2>/dev/null || true
        err "服务启动失败"
    fi
}

# ─── uninstall ───────────────────────────────────────
do_uninstall() {
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
    detect_binary

    [[ -f "/etc/systemd/system/${SERVICE_NAME}" ]] \
        || err "服务未安装，无法 update。请先运行: sudo $0 install"

    # hash 比对：无变化则跳过重启
    if [[ -f "${INSTALL_DIR}/${APP_NAME}" ]] && command -v sha256sum >/dev/null 2>&1; then
        local new_hash old_hash
        new_hash=$(sha256sum "$BINARY_SRC"             | awk '{print $1}')
        old_hash=$(sha256sum "${INSTALL_DIR}/${APP_NAME}" | awk '{print $1}')
        if [[ "$new_hash" == "$old_hash" ]]; then
            ok "二进制 SHA256 无变化，跳过重启"
            return 0
        fi
    fi

    info "停止服务"
    systemctl stop "$SERVICE_NAME"

    info "替换二进制"
    install -m 0755 -o "$RUN_USER" -g "$RUN_USER" "$BINARY_SRC" "${INSTALL_DIR}/${APP_NAME}"

    info "重启服务"
    systemctl start "$SERVICE_NAME"

    sleep 1
    if systemctl is-active --quiet "$SERVICE_NAME"; then
        ok "更新完成，服务已重启"
        health_check
    else
        warn "服务未进入 active，最近 20 行日志:"
        journalctl -u "$SERVICE_NAME" -n 20 --no-pager 2>/dev/null || true
        err "服务启动失败"
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
        du -sh "${INSTALL_DIR}/data"      2>/dev/null || echo "  data/       (未创建)"
        du -sh "${INSTALL_DIR}/logs"      2>/dev/null || echo "  logs/       (未创建)"
    fi

    if command -v curl >/dev/null 2>&1; then
        echo ""
        if curl -fs --max-time 3 "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
            ok "/health 通过"
        else
            warn "/health 未通过"
        fi
    fi
}

# ─── 入口 ────────────────────────────────────────────
ACTION="${1:-install}"
case "$ACTION" in
    install|uninstall|update)
        escalate_if_needed "$@"
        preflight_check
        case "$ACTION" in
            install)   do_install ;;
            uninstall) do_uninstall ;;
            update)    do_update ;;
        esac
        ;;
    status)
        preflight_check
        do_status
        ;;
    -h|--help|help)
        echo "用法: $0 [install|uninstall|update|status]"
        echo ""
        echo "  (无参数)    一键安装并启动（已安装则自动 update）"
        echo "  install     同上"
        echo "  uninstall   停止并卸载服务（询问是否删数据/用户）"
        echo "  update      原地更新二进制（hash 无变化则跳过重启）"
        echo "  status      查看服务状态 + 健康检查"
        ;;
    *)
        err "未知动作: $ACTION（用 -h 查看帮助）"
        ;;
esac
