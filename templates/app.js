    // 加载动画
    window.addEventListener('load', function() {
        setTimeout(function() {
            var loader = document.getElementById('loader');
            loader.style.opacity = '0';
            setTimeout(function() { loader.style.display = 'none'; }, 400);
        }, 600);
    });

    // Cookie 管理
    (function() {
        var overlay = document.getElementById('cookie-overlay');
        var msg = document.getElementById('cookie-msg');
        var title = document.getElementById('cookie-title');
        var saveBtn = document.getElementById('cookie-save-btn');
        var input = document.getElementById('cookie-input');
        var cookieValid = false;

        function applyValidState() {
            title.textContent = 'Cookie 已配置';
            saveBtn.disabled = true;
            saveBtn.style.opacity = '0.4';
            input.value = '';
        }

        function applyInvalidState() {
            title.textContent = '请配置网易云 Cookie';
            saveBtn.disabled = false;
            saveBtn.style.opacity = '';
        }

        function showOverlay() {
            overlay.classList.remove('hidden');
            msg.textContent = '';
            if (cookieValid) { applyValidState(); } else { applyInvalidState(); }
        }
        function hideOverlay() {
            overlay.classList.add('hidden');
            input.value = '';
        }

        // textarea 输入时：cookie 有效则解锁保存按钮
        input.addEventListener('input', function() {
            if (cookieValid && input.value.trim()) {
                saveBtn.disabled = false;
                saveBtn.style.opacity = '';
            } else if (cookieValid && !input.value.trim()) {
                saveBtn.disabled = true;
                saveBtn.style.opacity = '0.4';
            }
        });

        // 页面加载时检查 Cookie 状态
        fetch('/cookie/status').then(function(r) { return r.json(); }).then(function(d) {
            if (d.data && d.data.cookie_status === 'valid') {
                cookieValid = true;
            } else {
                cookieValid = false;
                showOverlay();
            }
        }).catch(function() {});

        // 设置按钮
        document.getElementById('settings-btn').addEventListener('click', showOverlay);

        // 跳过按钮
        document.getElementById('cookie-skip-btn').addEventListener('click', hideOverlay);

        // 保存按钮
        saveBtn.addEventListener('click', function() {
            var val = input.value.trim();
            if (!val) {
                msg.textContent = 'Cookie 不能为空';
                msg.className = 'cookie-msg cookie-msg-err';
                return;
            }
            saveBtn.disabled = true;
            saveBtn.textContent = '保存中...';
            msg.textContent = '';

            fetch('/cookie', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ cookie: val })
            }).then(function(r) { return r.json(); }).then(function(d) {
                if (d.data && d.data.cookie_status === 'valid') {
                    cookieValid = true;
                    msg.textContent = 'Cookie 验证通过';
                    msg.className = 'cookie-msg cookie-msg-ok';
                    setTimeout(function() { hideOverlay(); applyValidState(); }, 800);
                } else {
                    msg.textContent = d.message || 'Cookie 验证未通过';
                    msg.className = 'cookie-msg cookie-msg-err';
                    saveBtn.disabled = false;
                    saveBtn.style.opacity = '';
                }
            }).catch(function() {
                msg.textContent = '网络错误，请重试';
                msg.className = 'cookie-msg cookie-msg-err';
                saveBtn.disabled = false;
                saveBtn.style.opacity = '';
            }).finally(function() {
                saveBtn.textContent = '保存';
            });
        });
    })();

    var _apInstance = null;

    // 统计栏 → 已迁 htmx 轮询（#stats-bar hx-get=/ui/stats hx-trigger="every 3s"）。
    // 原 EventSource SSE 消费已删；服务端 /ui/stats 渲 stats_bar 片段。

    // 通用拖拽手柄
    function makeDraggable(handleId, onDrag) {
        var handle = document.getElementById(handleId);
        if (!handle) return;
        var dragging = false, startY, startVal;
        function begin(y) {
            dragging = true; startY = y; startVal = onDrag.getVal();
            handle.classList.add('active');
            document.body.style.cursor = 'row-resize';
            document.body.style.userSelect = 'none';
        }
        function move(y) {
            if (!dragging) return;
            var delta = y - startY;
            onDrag.setVal(Math.max(onDrag.min, startVal + delta));
        }
        function end() {
            if (!dragging) return;
            dragging = false;
            handle.classList.remove('active');
            document.body.style.cursor = '';
            document.body.style.userSelect = '';
        }
        handle.addEventListener('mousedown', function(e) { begin(e.clientY); e.preventDefault(); });
        document.addEventListener('mousemove', function(e) { move(e.clientY); });
        document.addEventListener('mouseup', end);
        handle.addEventListener('touchstart', function(e) { begin(e.touches[0].clientY); }, { passive: true });
        document.addEventListener('touchmove', function(e) { if (dragging) move(e.touches[0].clientY); });
        document.addEventListener('touchend', end);
    }

    // 歌词区拖拽 — 改 .lyric-box 高度
    makeDraggable('lyric-handle', {
        min: 60,
        getVal: function() { return document.getElementById('lyric').offsetHeight; },
        setVal: function(h) { document.getElementById('lyric').style.height = h + 'px'; }
    });

    // 播放器区拖拽 — 同时改 .aplayer-lrc 和 .aplayer 高度
    makeDraggable('player-handle', {
        min: 40,
        getVal: function() {
            var lrc = document.querySelector('.player-section .aplayer-lrc');
            return lrc ? lrc.offsetHeight : 70;
        },
        setVal: function(h) {
            var lrc = document.querySelector('.player-section .aplayer-lrc');
            var ap = document.querySelector('.player-section .aplayer');
            if (!lrc || !ap) return;
            var delta = h - lrc.offsetHeight;
            lrc.style.height = h + 'px';
            ap.style.height = (ap.offsetHeight + delta) + 'px';
        }
    });

    // Toast 提示
    function showToast(msg, isError) {
        var t = document.getElementById('toast');
        t.textContent = msg;
        t.className = 'toast' + (isError ? ' error' : '');
        setTimeout(function() { t.classList.add('show'); }, 10);
        setTimeout(function() { t.classList.remove('show'); }, 2500);
    }

    // ========== 全局下载锁：单用户同时只允许一个下载任务 ==========
    var _dlLocked = false;
    var _dlBtnSelector = '#download-btn, #detail-download-btn, .download-song, #playlist-download-all, #album-download-all';
    function dlLock() {
        _dlLocked = true;
        $(_dlBtnSelector).prop('disabled', true).addClass('dl-locked');
    }
    function dlUnlock() {
        _dlLocked = false;
        $(_dlBtnSelector).prop('disabled', false).removeClass('dl-locked');
    }

    // 浮动进度条
    function showDlProgress(title) {
        $('#dl-float-title').text(title || '正在下载');
        $('#dl-float-pct').text('0%');
        $('#dl-float-fill').css('width', '0%');
        $('#dl-float-detail').text('准备中...');
        $('#dl-float').addClass('show');
    }
    function updateDlProgress(pct, detail) {
        if (pct !== undefined) {
            $('#dl-float-pct').text(pct + '%');
            $('#dl-float-fill').css('width', pct + '%');
        }
        if (detail) $('#dl-float-detail').text(detail);
    }
    function hideDlProgress() {
        $('#dl-float').removeClass('show');
    }

    // 通用：轮询进度 → 拿结果
    var _pollInterval = null;
    var _pollStopped = false;
    var _currentTaskId = null;
    function pollAndFetch(taskId, $btn, btnText) {
        _currentTaskId = taskId;
        if (_pollInterval) clearInterval(_pollInterval);
        _pollStopped = false;
        var netFailCount = 0;
        var taskLostCount = 0;
        var lastDetail = '';
        console.log('[poll] start polling taskId=' + taskId);
        function finish(reason) {
            console.log('[poll] finish reason=' + reason);
            _pollStopped = true;
            clearInterval(_pollInterval); _pollInterval = null;
            _currentTaskId = null;
        }
        function restore() {
            if ($btn) $btn.text(btnText);
            dlUnlock();
        }
        _pollInterval = setInterval(function() {
            if (_pollStopped) return;
            $.ajax({
                url: '/download/progress/' + taskId,
                method: 'GET', cache: false, dataType: 'json',
                timeout: 10000,
                success: function(resp) {
                    if (_pollStopped) return;
                    netFailCount = 0;
                    if (!resp.success) {
                        taskLostCount++;
                        console.log('[poll] resp not success, taskLostCount=' + taskLostCount, resp);
                        if (taskLostCount >= 15) {
                            console.log('[poll] HIDE: task lost after 15 failures');
                            finish('task_lost'); hideDlProgress();
                            showToast('任务丢失，请重试', true);
                            restore();
                        }
                        return;
                    }
                    taskLostCount = 0;
                    var d = resp.data;
                    lastDetail = d.detail || lastDetail;
                    updateDlProgress(d.percent, d.detail);
                    if (d.stage === 'done' || d.stage === 'retrieved') {
                        console.log('[poll] HIDE: stage=' + d.stage + ', triggering download');
                        finish('done');
                        updateDlProgress(100, '开始下载...');
                        var a = document.createElement('a');
                        a.href = '/download/result/' + taskId;
                        a.style.display = 'none';
                        document.body.appendChild(a);
                        a.click();
                        document.body.removeChild(a);
                        showToast('下载已开始，请查看浏览器下载栏');
                        hideDlProgress();
                        restore();
                    } else if (d.stage === 'error') {
                        console.log('[poll] HIDE: stage=error, error=' + (d.error || ''));
                        finish('error'); hideDlProgress();
                        showToast('下载失败: ' + (d.error || '未知错误'), true);
                        restore();
                    }
                },
                error: function(xhr, status, err) {
                    if (_pollStopped) return;
                    if (xhr.status === 404) {
                        taskLostCount++;
                        console.log('[poll] task 404 #' + taskLostCount);
                        if (taskLostCount >= 15) {
                            console.log('[poll] HIDE: task lost (404) after 15 failures');
                            finish('task_lost'); hideDlProgress();
                            showToast('任务不存在或已过期，请重试', true);
                            restore();
                        }
                        return;
                    }
                    netFailCount++;
                    console.log('[poll] ajax error #' + netFailCount, 'status=' + status, 'err=' + err);
                    updateDlProgress(undefined, '网络异常，重连中 (' + netFailCount + ')...');
                    if (netFailCount >= 60) {
                        console.log('[poll] HIDE: network fail after 60 errors');
                        finish('net_fail'); hideDlProgress();
                        showToast('网络持续异常，请检查网络后重试', true);
                        restore();
                    }
                }
            });
        }, 800);
    }

    // 取消下载按钮
    $('#dl-float-cancel').on('click', function() {
        if (!_currentTaskId) return;
        var tid = _currentTaskId;
        console.log('[poll] HIDE: user cancelled, taskId=' + tid);
        $.ajax({
            url: '/download/cancel/' + tid,
            method: 'POST', dataType: 'json',
            success: function() {},
            error: function() {}
        });
        _pollStopped = true;
        if (_pollInterval) { clearInterval(_pollInterval); _pollInterval = null; }
        _currentTaskId = null;
        hideDlProgress();
        dlUnlock();
        showToast('已取消下载');
    });

    // 大图预览
    function showBigPic(src) {
        document.getElementById('modal-pic').src = src;
        document.getElementById('picModal').classList.add('show');
    }

    $(document).ready(function() {
        // 标签切换
        var areas = ['search','parse','playlist','album','download'];
        $('#tab-nav').on('click', 'button', function() {
            var tab = $(this).data('tab');
            $('#tab-nav button').removeClass('active');
            $(this).addClass('active');
            areas.forEach(function(a) {
                $('#' + a + '-area')[a === tab ? 'removeClass' : 'addClass']('area-hidden');
            });
            // 切换时隐藏不相关的结果
            if (tab !== 'search') $('#search-result').addClass('area-hidden');
            if (tab !== 'parse') $('#song-info').addClass('area-hidden');
            if (tab !== 'playlist') $('#playlist-result').addClass('area-hidden');
            if (tab !== 'album') $('#album-result').addClass('area-hidden');
        });

        // 歌词处理
        function lrctrim(lyrics) {
            var lines = lyrics.split('\n'), data = [];
            lines.forEach(function(line, index) {
                var m = line.match(/\[(\d{2}):(\d{2}[\.:]?\d*)]/);
                if (m) {
                    var mins = parseInt(m[1], 10);
                    var secs = parseFloat(m[2].replace('.', ':')) || 0;
                    var ts = mins * 60000 + secs * 1000;
                    var text = line.replace(/\[\d{2}:\d{2}[\.:]?\d*\]/g, '').trim().replace(/\s\s+/g, ' ');
                    data.push([ts, index, text]);
                }
            });
            data.sort(function(a, b) { return a[0] - b[0]; });
            return data;
        }

        function lrctran(lyric, tlyric) {
            lyric = lrctrim(lyric);
            tlyric = lrctrim(tlyric);
            for (var i = 0, j = 0; i < lyric.length && j < tlyric.length; i++) {
                while (lyric[i][0] > tlyric[j][0] && j + 1 < tlyric.length) j++;
                if (lyric[i][0] === tlyric[j][0]) {
                    tlyric[j][2] = tlyric[j][2].replace('/', '');
                    if (tlyric[j][2]) lyric[i][2] += ' (' + tlyric[j][2] + ')';
                    j++;
                }
            }
            var result = '';
            for (var k = 0; k < lyric.length; k++) {
                var t = lyric[k][0];
                result += '[' + String(Math.floor(t / 60000)).padStart(2, '0') + ':' +
                    String(Math.floor((t % 60000) / 1000)).padStart(2, '0') + '.' +
                    String(t % 1000).padStart(3, '0') + ']' + lyric[k][2] + '\n';
            }
            return result;
        }

        // songItemHTML / escapeHtml 已移除 —— 歌曲列表项改服务端 Maud 渲染
        // （view::components::song_item，自动 HTML 转义，XSS 防护更稳）。

        // 搜索 → 已迁 htmx（#search-btn hx-post=/ui/search hx-target=#search-result）。
        // 点击即时反馈由 .htmx-request CSS 承载（不变量 D），区域 outerHTML swap 不整页刷新。

        // 搜索结果点击解析（select-song 填 ID + 切 tab，仍归 JS；不与 htmx 冲突）
        $(document).on('click', '.select-song', function() {
            var id = $(this).data('id');
            $('#song_ids').val(id);
            $('#tab-nav button[data-tab="parse"]').trigger('click');
            $('html,body').animate({scrollTop: 0}, 300);
        });

        // ========== 统一下载函数（3阶段：启动 → 轮询进度 → 获取结果） ==========
        function downloadZip(musicId, quality, triggerBtn, cachedMeta) {
            if (_dlLocked) { showToast('已有下载任务进行中', true); return; }
            quality = quality || 'lossless';
            var $btn = triggerBtn ? $(triggerBtn) : null;
            var btnText = $btn ? $btn.text() : '';

            dlLock();
            if ($btn) $btn.text('下载中...');
            showDlProgress('正在下载');

            var body = { id: musicId, quality: quality };
            if (cachedMeta && cachedMeta.id && String(cachedMeta.id) === String(musicId)) {
                body.name = cachedMeta.name;
                body.artists = cachedMeta.artists;
                body.album = cachedMeta.album;
                body.pic_url = cachedMeta.pic_url;
                body.lyric = cachedMeta.lyric;
                body.tlyric = cachedMeta.tlyric;
            }

            $.ajax({
                url: '/download/start', method: 'POST',
                contentType: 'application/json',
                data: JSON.stringify(body), dataType: 'json',
                success: function(resp) {
                    if (resp.success) {
                        pollAndFetch(resp.data.task_id, $btn, btnText);
                    } else {
                        hideDlProgress();
                        showToast(resp.message || '启动失败', true);
                        if ($btn) $btn.text(btnText);
                        dlUnlock();
                    }
                },
                error: function() {
                    hideDlProgress();
                    showToast('启动下载失败', true);
                    if ($btn) $btn.text(btnText);
                    dlUnlock();
                }
            });
        }

        // 记住当前解析的歌曲 ID 和元数据
        var currentParsedId = '';
        var currentParsedMeta = null;

        // 单曲解析 → htmx（#parse-btn hx-post=/ui/song hx-target=#song-detail-body
        // hx-swap=innerHTML）。服务端只直出「详情卡内层」；歌词区 / 播放器区是 page_shell
        // 的持久节点，htmx 永不 swap → #aplayer 单一声明、容器恒在。
        //
        // 【单源编排 · 根治多源竞态，勿改回旧结构】旧版片段每次解析都重建 #aplayer，
        // afterSwap 里 APlayer 给容器加的 `.aplayer` 类被 htmx settle 重置回片段原值
        // （class=""）→ 所有 `.aplayer` scope CSS（`svg{width:100%}` / `.aplayer-icon-play
        // {display:none}`）失配 → 控制图标渲染成铺满屏幕的巨型三角。现把 #aplayer 移出 swap
        // 目标（详见 page_shell #song-info > #song-detail-body 结构），htmx 不再碰它，类稳定。
        //
        // 触发判据：afterSettle 且 swap 目标恰为 #song-detail-body（精确，stats 轮询等无关 swap
        // 自动跳过；无需 dataset.done 兜底）。下载优化元数据 + 歌词合并(lrctran JS 岛)一并承接。
        document.body.addEventListener('htmx:afterSettle', function(evt) {
            if (!evt.target || evt.target.id !== 'song-detail-body') return;
            var card = document.getElementById('song-info');
            var lyricSec = document.getElementById('lyric-section');
            var playerSec = document.getElementById('player-section');
            if (card) card.classList.remove('area-hidden'); // 成功 / 错误都显示详情卡

            var metaEl = document.getElementById('parsed-meta');
            var d = null;
            if (metaEl) { try { d = JSON.parse(metaEl.textContent); } catch (e) { d = null; } }

            // 错误 / 无数据：拆掉旧播放器，隐藏歌词与播放器区（仅显示错误文案）
            if (!d) {
                if (_apInstance) { try { _apInstance.destroy(); } catch (e) {} _apInstance = null; }
                if (lyricSec) lyricSec.classList.add('area-hidden');
                if (playerSec) playerSec.classList.add('area-hidden');
                return;
            }

            // 下载优化全局（#detail-download-btn 复用：避免重解析 + 携带歌词打 tag）
            currentParsedId = d.id;
            currentParsedMeta = {
                id: d.id, name: d.name, artists: d.ar_name, album: d.al_name,
                pic_url: d.pic, lyric: d.lyric || '', tlyric: d.tlyric || '', quality: d.level
            };

            // 歌词合并 + 显示
            var lrc = d.lyric || '';
            if (d.tlyric) lrc = lrctran(d.lyric, d.tlyric);
            $('#lyric').html(lrc.replace(/\n/g, '<br>'));
            if (lyricSec) lyricSec.classList.remove('area-hidden');

            // APlayer 初始化（持久 #aplayer，htmx 不碰它 → `.aplayer` 类不被抹）
            if (playerSec) playerSec.classList.remove('area-hidden');
            if (_apInstance) { try { _apInstance.destroy(); } catch (e) {} _apInstance = null; }
            var apEl = document.getElementById('aplayer');
            if (apEl) {
                apEl.innerHTML = '';
                _apInstance = new APlayer({
                    container: apEl, lrcType: 1,
                    audio: [{ name: d.name, artist: d.ar_name, url: d.url, cover: d.pic, lrc: lrc }]
                });
                // 重置拖拽留下的 inline style
                var apDiv = document.querySelector('.player-section .aplayer');
                if (apDiv) apDiv.style.height = '';
                var lrcDiv = document.querySelector('.player-section .aplayer-lrc');
                if (lrcDiv) lrcDiv.style.height = '';
            }
        });

        // 详情卡片"原始链接"按钮 → 直接跳转CDN链接
        $(document).on('click', '#detail-direct-btn', function() {
            var url = $(this).data('url');
            if (!url) { showToast('无下载链接', true); return; }
            var a = document.createElement('a');
            a.href = url;
            a.download = $(this).data('filename') || '';
            a.target = '_blank';
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
        });

        // 详情卡片"下载完整包"按钮 → 优先使用缓存元数据
        $(document).on('click', '#detail-download-btn', function() {
            var id = $(this).data('id') || currentParsedId;
            var quality = $(this).data('quality') || $('#level').val() || 'lossless';
            if (!id) { showToast('请先解析歌曲', true); return; }
            downloadZip(id, quality, this, currentParsedMeta);
        });

        // 歌单 / 专辑解析 → 已迁 htmx（#playlist-btn /ui/playlist、#album-btn /ui/album，
        // outerHTML 区域 swap）。URL→id 抽取 + 类型误投提示移至服务端。点击反馈走 .htmx-request CSS。

        // ========== 批量下载 Tab ==========
        $('#download-btn').on('click', function() {
            var raw = $('#download_id').val().trim();
            if (!raw) { showToast('请输入音乐 ID 或 URL', true); return; }
            var lines = raw.split(/[\n\r]+/).map(function(s) { return s.trim(); }).filter(function(s) { return s; });
            if (lines.length === 0) { showToast('请输入音乐 ID 或 URL', true); return; }
            if (lines.length > 100) { showToast('单次最多下载100首', true); return; }
            var quality = $('#download_quality').val();

            if (lines.length === 1) {
                var mid = lines[0];
                var m = mid.match(/song\?id=(\d+)/);
                if (m) mid = m[1];
                downloadZip(mid, quality, this);
            } else {
                downloadBatch(lines, quality, this);
            }
        });

        // 列表"下载"按钮 → 直接走服务器下载 ZIP
        $(document).on('click', '.download-song', function() {
            var id = $(this).data('id');
            if (!id) { showToast('无法获取音乐 ID', true); return; }
            downloadZip(id, 'lossless', this);
        });

        // 列表"添加"按钮 → 追加 ID 到下载 textarea + 封面飞入动画
        $(document).on('click', '.add-to-batch', function() {
            var id = String($(this).data('id'));
            if (!id) return;

            // 去重：检查 textarea 中是否已有
            var ta = document.getElementById('download_id');
            var existing = ta.value.split(/[\n\r]+/).map(function(s) { return s.trim(); }).filter(function(s) { return s; });
            if (existing.indexOf(id) !== -1) {
                showToast('已在下载列表中', true);
                return;
            }

            // 封面飞入动画
            var $cover = $(this).closest('.song-item').find('.song-item-cover');
            var $target = $('#tab-nav button[data-tab="download"]');
            if ($cover.length && $target.length) {
                var srcRect = $cover[0].getBoundingClientRect();
                var tgtRect = $target[0].getBoundingClientRect();
                var $fly = $('<img class="fly-cover">').attr('src', $cover.attr('src'));
                $fly.css({
                    left: srcRect.left + 'px', top: srcRect.top + 'px',
                    width: srcRect.width + 'px', height: srcRect.height + 'px',
                    opacity: 1
                });
                $('body').append($fly);
                requestAnimationFrame(function() {
                    $fly.css({
                        left: (tgtRect.left + tgtRect.width / 2 - 10) + 'px',
                        top: (tgtRect.top + tgtRect.height / 2 - 10) + 'px',
                        width: '20px', height: '20px',
                        opacity: 0, borderRadius: '50%'
                    });
                });
                setTimeout(function() { $fly.remove(); }, 600);
            }

            // 追加 ID
            var val = ta.value.trim();
            ta.value = val ? val + '\n' + id : id;
            showToast('已添加到下载列表');
        });

        // ========== 批量下载（3阶段） ==========
        function downloadBatch(ids, quality, triggerBtn) {
            if (_dlLocked) { showToast('已有下载任务进行中', true); return; }
            quality = quality || 'lossless';
            if (!ids || ids.length === 0) { showToast('没有可下载的曲目', true); return; }
            if (ids.length > 100) { showToast('单次最多下载100首', true); return; }

            var $btn = triggerBtn ? $(triggerBtn) : null;
            var btnText = $btn ? $btn.text() : '';
            dlLock();
            if ($btn) $btn.text('批量下载中...');
            showDlProgress('批量下载 (' + ids.length + '首)');

            $.ajax({
                url: '/download/batch/start', method: 'POST',
                contentType: 'application/json',
                data: JSON.stringify({ ids: ids, quality: quality }), dataType: 'json',
                success: function(resp) {
                    if (resp.success) {
                        pollAndFetch(resp.data.task_id, $btn, btnText);
                    } else {
                        hideDlProgress();
                        showToast(resp.message || '启动失败', true);
                        if ($btn) $btn.text(btnText);
                        dlUnlock();
                    }
                },
                error: function() {
                    hideDlProgress();
                    showToast('启动批量下载失败', true);
                    if ($btn) $btn.text(btnText);
                    dlUnlock();
                }
            });
        }

        // 歌单"下载全部"
        $(document).on('click', '#playlist-download-all', function() {
            var ids = [];
            $('#playlist-tracks .download-song').each(function() {
                ids.push($(this).data('id'));
            });
            var quality = $('#playlist-quality').val() || 'lossless';
            downloadBatch(ids, quality, this);
        });

        // 专辑"下载全部"
        $(document).on('click', '#album-download-all', function() {
            var ids = [];
            $('#album-tracks .download-song').each(function() {
                ids.push($(this).data('id'));
            });
            var quality = $('#album-quality').val() || 'lossless';
            downloadBatch(ids, quality, this);
        });

        // Enter 键触发
        // 已迁 htmx 的按钮用原生 .click() 确保触发 htmx（jQuery .click() 对原生监听有歧义）
        $('#search_keywords').on('keydown', function(e) { if (e.key === 'Enter') document.getElementById('search-btn').click(); });
        $('#song_ids').on('keydown', function(e) { if (e.key === 'Enter') document.getElementById('parse-btn').click(); });
        $('#playlist_id').on('keydown', function(e) { if (e.key === 'Enter') document.getElementById('playlist-btn').click(); });
        $('#album_id').on('keydown', function(e) { if (e.key === 'Enter') document.getElementById('album-btn').click(); });

        // 管理面板：#admin-btn 已在 page_shell 接 htmx（hx-get /ui/admin）；
        // 登录/Enter 由服务端 <form> 原生处理（无需 JS 绑定）。
    });

    // ============ 管理面板 JS 岛（仅遮罩关闭 + token 头注入）============
    // 登录/设置/配置 CRUD/校验/单位换算 全在服务端 /ui/admin/*；这里只剩两件浏览器专属事。

    // 关闭遮罩（admin 片段里的 × 按钮 onclick 调用，须全局）
    function closeAdmin() {
        document.getElementById('admin-overlay').classList.add('hidden');
    }

    // token 头注入：/ui/admin/* 请求带上配置视图 data-token 的 X-Admin-Token
    // （token 登录后存于 #admin-config-view[data-token]，随 DOM 替换而生灭）
    document.body.addEventListener('htmx:configRequest', function(evt) {
        var p = evt.detail.path || '';
        if (p.indexOf('/ui/admin/') === 0) {
            var cv = document.getElementById('admin-config-view');
            if (cv && cv.dataset.token) {
                evt.detail.headers['X-Admin-Token'] = cv.dataset.token;
            }
        }
    });

