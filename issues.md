# Denial 渲染管线与输入管线 gap 审核清单

> 全量审核日期：2026-08-19（输入路由 z-test + 组合区域重构完成后）
> 审核维度：坐标映射 / 绘制-输入顺序一致性 / 纹理采样可见性 / 窗口几何装饰
> 状态图例：⬜ 待修 / ✅ 已修 / 🔒 已评估不需修

---

## ✅ 已修（2026-08-20 人类模拟测试发现）

- [x] **H6 — move 后 ConfigureWindow 永不发送 → resize grab 把窗口拉回旧位置**
  - 位置：`desktop_input_layout_publisher.dart`（`DesktopWindowConfigureTracker.update`）
  - 问题：拖动中（`nativeDragActive` 分支）return null 但 `_configured[objectId] = geometry` 已执行（"last seen" 与 "sent" 混淆）；拖动结束后 `previous == geometry` 触发去重 skip → ConfigureWindow 永不发出 → Rust `space.element_location` 冻结在旧值 → 下次原生 resize grab 以旧位置为基准（`begin_super_pointer_grab` 的 `initial_location = space.element_location`），窗口被拉回 move 前位置。
  - 复现：标题栏拖动窗口到新位置 → 任意边 press（不拖）→ 窗口跳回 move 前位置。
  - 修复：仅在真正发送时更新 `_configured`（drag 分支不再存值），move 结束后最终值正常发送。验证：move→press 不再拉回（16/16 人类模拟 PASS）。
  - 注：`handlers.rs:377` 的 `!exact && Resizing → ignore configure` 保护与此无关（grab 结束 `finish()` 已 unset Resizing）。

---

## 🔴 高严重性

- [ ] **H1 — decoration 检查不按 z 深度序**
  - 位置：`compositor/src/bin/deniald/wayland_frontend/input.rs:710-723`
  - 问题：decoration（SSD 标题栏）用 `.any()` 检查**所有**窗口，先于窗口 region 列表且不做 z 比较（注释声称 depth-tested，实现不符）。高层窗口内容覆盖低层标题栏时点击被抢路由到 shell，高层窗口收不到点击；popup 覆盖标题栏时同样被抢（IM popup 也受影响）。
  - 建议：decoration 并入 windows 的 front-to-back 短路遍历（同一深度单元：命中某窗口 region 后，若其 decoration 含该点则路由 shell，否则路由该窗口）。

- [ ] **H2 — 动画期间输入 region 与绘制几何不同源**
  - 位置：`dart_shell/lib/src/desktop/desktop_shell.dart:1013-1021,2469-2470,2697-2705`（AnimatedPositioned 插值）、`desktop_input_layout_publisher.dart:195-243`（region 一律用静态 placement.contentRect）
  - 问题：overview/switcher 退出动画（280ms）、maximize/restore、minimize→widget restore 期间绘制从旧帧动画到新帧，输入 region 在同一 post-frame 立即跳到目标几何。动画中点击视觉位置落空或命中邻居/下层窗口。
  - 建议：fullScene 保持到 exit 动画结束（`active` 覆盖 exit phase）；或 publisher 读动画当前值发布插值几何；maximize/restore 需 "settle 后再发布" 机制。

- [ ] **H3 — SSD 窗口 popup 绘制比输入区高 36px**
  - 位置：`dart_shell/lib/src/desktop/desktop_shell.dart:2262-2266`（`_DesktopPopupSurfaceLayers` 用 `frame.deflate(frameBorder)` 只减 1px，缺 36px 标题栏）；输入侧 `desktop_input_layout_publisher.dart:213` 用 `placement.contentRect`（含 36px）
  - 问题：SSD 窗口（GTK/Electron 等）的菜单/工具提示画在正确位置上方 36px：上半截叠在标题栏上（点击被装饰区抢走），下半截输入区落在无视觉内容处。
  - 建议：popup 绘制改用与主内容相同的 inset（`placement.contentRect`），与输入侧共用同一派生函数。

- [ ] **H4 — Rust 作者的最大化几何缺 36px 标题栏**
  - 位置：`compositor/src/bin/deniald/wayland_frontend/window_management.rs:108-123`（`shell_content_geometry` 只减 1px 边框）、`:934-935`（SUPER+Up maximize）、`:1079-1080`（垂直最大化）
  - 问题：Rust 全库无标题栏高度概念（`SHELL_FRAME_BORDER=1` 有，`title_bar` 0 命中），与 Dart `titleBarHeight=36` 分歧。Rust 先发错误尺寸 configure → Dart 纠正 → 客户端两次 configure、可能按错误尺寸 commit（纹理压扁）、placement 事件膨胀成 frame 越出 workArea 36px。
  - 建议：Rust 引入 `SHELL_TITLE_BAR_HEIGHT=36`（与 DesktopMetrics 双注释镜像），`shell_content_geometry` 对 SSD 顶部再减 36；或 SUPER+Up 路径直接复用 Dart 已作者的 rect。

- [ ] **H5 — IM popup surface 永远进不了 visible 集 → 输入法候选窗冻结**
  - 位置：`wayland_frontend.rs:3557-3583`（`install_input_layout` 的 `root_ids` 只由 `space.elements()` 根 surface 构建）、`wayland_frontend.rs:2406-2414`（`toplevel_candidate_surface` 只认 Xdg popup，IM popup 在 `input_method.rs` 自己的表里）、`wayland_frontend.rs:3405-3409`（`expects_sample` 恒 false → buffer 提前 release 撕裂风险）、`wayland_frontend.rs:3839-3843`（frame callback 门恒 false → **fcitx5 候选窗冻结**）
  - 建议：`install_input_layout` 对 `input_method.visible_popups()` 的 surface 直接 insert 其 stable_id；或 `toplevel_candidate_surface` 同时查 input_method 登记表。

---

## 🟡 中严重性

- [ ] **M1 — 绝对输入（触摸/绝对指针）未应用输出 transform**
  - 位置：`input.rs:1949-1950`（绝对指针）、`input.rs:2211-2213`（触摸）；对照 `topology.rs:101-107`（atlas 已按 transform 交换宽高）、KMS scanout 旋转
  - 问题：旋转 90°/270° 输出上，绝对设备只做范围映射未做旋转反变换 → 落点相对视觉错位；`topology.rs:382-386` 的 `touch_bounds` 只取第一个输出。
  - 建议：绝对输入按命中输出的 OutputTransform 做反变换；`touch_bounds` 覆盖全部输出包围盒。

- [ ] **M2 — rect/sourceRect 1:1 不变量 Rust 侧无校验**
  - 位置：`wire.rs:1739-1746,1805-1822`、`input.rs:389-390`（scale 兜底静默传播错误）
  - 问题：历史 bug 根因正是尺寸不一致 → 比例缩放错位；修复靠 Dart 侧约定，Rust 无校验/警告，任何未来回归会再次静默错位。
  - 建议：decode 校验 `|rect.w−source.w|`/`|rect.h−source.h|` 容差内（或 warn!）；`map_to` 文档注释声明支持语义（1:1 或按轴均匀拉伸）。

- [ ] **M3 — popup sourceRect 原点假设与 mapSurfaceRect 公式不等价**
  - 位置：`desktop_input_layout_publisher.dart:212-229` + `denial_window.dart:284-300`（`mapSurfaceRect`）
  - 问题：popup `sourceRect=Rect(0,0,w,h)` 假设"原点即 popupRect 原点"；当 content_x≠0（xdg geometry 偏移非零，如 CSD 阴影窗口）与 k≠1 组合时整体偏移 content_x·k。
  - 建议：popup sourceRect 显式含 content_x 偏移（与 mapSurfaceRect 同一推导）；或 Rust 对 popup region 校验尺寸比与窗口一致。

- [ ] **M4 — overview 激活期发布原始 frame 而非 overview frames**
  - 位置：`desktop_input_layout_publisher.dart:100-110,125-132,200-243`（用 placement.frame）对照 `desktop_workspace.dart:295`（绘制用 `overview?.frames`）、`desktop_overview_layout.dart:43-75`（预览重排缩放）
  - 问题：预览被缩放移动，region rect 却是原始 frame：预览与原始 rect 相交 → 点击比例映射路由给客户端（隐式可点）；不相交 → 落入 shellRegions。同一视觉位置行为取决于是否碰巧相交。
  - 建议：overview 期间发布 overview frames 作 region rect（sourceRect 用预览缩放推导），或预览区显式加入 shellRegions 统一为 shell 处理。

- [ ] **M5 — 系统栏 media 控件 region 绘制在窗口下层却无条件优先**
  - 位置：`desktop_system_bar.dart:362-364`（childBounds）、`desktop_input_layout_publisher.dart:134-139`（无条件加入 shellRegions）、`input.rs:693-700`（shell 先于窗口）、`desktop_shell.dart:1363-1368`（系统栏绘制在窗口平面之下）
  - 问题：一般不变式 "childRegions 均绘制于窗口之上" 对 launcher/dashboard 成立，但系统栏 media 按钮在 wallpaper 平面；窗口拖到栏上时绘制是窗口盖住按钮，输入却是按钮命中。
  - 建议：publisher 对 childRegions 做窗口 frame 裁剪，或 media 控件 region 提升到窗口之上。

- [ ] **M6 — 无遮挡剔除：被完全遮挡窗口仍在采样**
  - 位置：`desktop_input_layout_publisher.dart:100-110,194`（不按遮挡过滤）、`desktop_shell.dart:963-1045`（全 placement 建帧无 culling）、`wayland_frontend.rs:3831-3837`（frame callback 只按 output membership）
  - 问题：被遮挡窗口仍 GPU 采样（opacity<1 还叠加 blur）、仍收 frame callback、expects_sample=true——违背渲染惯例。
  - 前提：遮挡者必须 `isOpaque`、shell 表面计入遮挡、capturesFullScene/overview/switcher 期间禁用、同步停被剔除窗口的 frame callback。
  - 建议：publisher 按 "上方存在 fully opaque 全盖窗口" 剔除 visibleSurfaceIds + 改 frame callback 门。

- [ ] **M7 — opacity=0 窗口仍被采样 → mailbox 阻塞**
  - 位置：`desktop_input_layout_publisher.dart:143-249`（无 opacity 判断）、`wayland_frontend.rs:2906-2910,3114-3118`（expects_sample 仍 true）、`flutter_runtime.rs:2159`（advance 阻塞）
  - 问题：opacity=0 的 X11 窗口场景不绘制但 expects_sample=true → 客户端 buffer 被钉住直至恢复可见。输入侧 opacity=0 保留 region 是正确惯例，仅采样侧浪费。
  - 建议：opacity==0 保留输入 region 但剔除 visibleSurfaceIds（或 Rust 侧 expects_sample=false）。

- [ ] **M8 — X11 无装饰协商：自绘装饰客户端被强制 SSD**
  - 位置：`window_management.rs:89-106`（`shell_draws_x11_server_frame` 不查 `_MOTIF_WM_HINTS`）、`wayland_frontend.rs:3217-3233`、`handlers.rs:1691-1702`（Wayland neutral 一律 SSD）
  - 问题：自绘装饰的 X11 客户端（Firefox/X11、部分 Java/Electron）被强制 SSD → 绘制双标题栏 + 输入侧顶部 37px 划给 shell → 客户端自己的控件不可点击。
  - 建议：X11 读 Motif/`_NET_WM` 装饰提示决定 flag；自绘客户端考虑"只画边框不画标题栏"降级模式。

- [ ] **M10 — Flutter 手势层丢失与 Up 同帧的最后一个 motion（引擎合并）**
  - 位置：Flutter 引擎 pointer event coalescing（每帧只保留最后一个事件）；影响所有 Flutter 手势（标题栏 move 等）
  - 问题：最后一个 motion 与 Up 落在同一引擎帧时被合并丢弃 → 手势 delta 少最后一段。真实鼠标影响 ≈1 次采样（1-3px），可忽略；uinput 稀疏事件（25-33px 步长）放大到 25-33% 损耗。
  - 建议：无需修（真实影响微小）；测试工具用 ≤10px 细步长规避（损耗降到 ~10%）。

- [ ] **M9 — resize 8 方向与可视边框不一致**
  - 位置：`input.rs:507-519`（`resize_edge_for_geometry` 只产 4 象限角）、`window_grab.rs:371-398`、`desktop_window_frame_painter.dart:41-50`（frame 全 IgnorePointer）、`mouse_cursor.rs:173-185`
  - 问题：SUPER+RMB 只有 4 角；顶边中点拖拽 = 角 resize（水平拖动同时改宽度，与预期相反）；无边缘 hover 光标提示。
  - 建议：SUPER+RMB/边缘 hover 按到四条边/四角的最近距离选边（保留 8 向语义）；frame 边缘加 MouseRegion 设 resize 光标。

---

## 🟢 低严重性

- [ ] **L1 — pinned 窗口 active chrome 与输入 topmost 不一致**
  - 位置：`desktop_shell.dart:1024`（`active = z == topZ`）、`desktop_workspace.dart:657-663`
  - 问题：pinned 窗口画在最上且输入首命中，但 z==topZ 不成立 → 显示未聚焦样式。点击一次后自愈。
  - 建议：active 判定改为 pinned-aware topmost（复用 compareDesktopWindowStack）。

- [ ] **L2 — minimized 桌面 widget 排序忽略 pinned**
  - 位置：`desktop_shell.dart:857-864`（按 z,objectId 排序）与 compareDesktopWindowStack 不一致。纯视觉（minimized 无输入 region）。
  - 建议：复用 compareDesktopWindowStack。

- [ ] **L3 — `ClientInputRoute.scene_origin` 冻结 vs atlas_origin 漂移**
  - 位置：`input.rs:321,380,391` vs `input.rs:692,786,799`、`topology.rs:387`
  - 问题：scene_origin==atlas_origin 是"恰好赋成一样"非强制不变量；atlas_origin 变化到新布局发布之间约一帧，缓存 route 用旧 origin 算客户端坐标。
  - 建议：route 记录 layout epoch，缓存命中校验；或 focus_at 现取 atlas_origin。

- [ ] **L4 — surface transform（wl_surface.set_buffer_transform）双端忽略**
  - 位置：`wire.rs:366-367,2011-2012`、`wayland_frontend.rs:2838`、`window_surface_tree.dart:61-133`（渲染不旋转）、`denial_wire.dart:820-875`
  - 问题：transform≠Normal 时 smithay dst 已交换宽高，但渲染拉伸不旋转、输入用未旋转 geometry——双端一致地错。当前无客户端使用，实际影响为零。
  - 建议：解码处对 transform≠Normal `warn!`（或拒绝）；未来支持时 map_to 同步做旋转反变换。

- [x] **L5 — 相对指针增量用全局 atlas_scale 归一** → **已并入 D1（升级为高）**
  - 位置：`input.rs:1924-1938`（delta/atlas_scale）对照 `wayland_frontend.rs:940`（atlas_scale=最大输出 scale）、`wire.rs:2707`（每输出独立 scale）
  - 问题：混合缩放输出（1.0+2.0）上 1.0 输出鼠标速度减半。
  - 建议：按指针所在输出 scale 归一。见 DPI 专项 D1。

- [ ] **L6 — focus_at 的 global_origin 第二次比例映射与 map_to 同构但独立演化**
  - 位置：`input.rs:381-384`（map_to）与 `input.rs:389-397`（scale_x/y + global_origin）
  - 问题：1:1 时无误差；非 1:1 时整数 subsurface 位置 ×scale 舍入放大（≤1px）。两处公式若被分别改动会静默偏移。
  - 建议：提取 "source→scene 比例映射" 为 map_to 逆运算共用 helper + 非 1:1 单测。

- [ ] **L7 — SUPER+RMB resize 不检查 maximized**
  - 位置：`input.rs:2964`（只查 geometry_locked）对照客户端 XDG resize 在 maximized/fullscreen 被拒（`handlers.rs:1311-1316`）
  - 问题：maximized 窗口可被 compositor resize → 客户端忽略 configure → 几何脱节、纹理拉伸。
  - 建议：begin_super_pointer_grab 对 maximized 拒绝或先解除 maximize。

- [ ] **L8 — 拖动 maximized 窗口丢失 restore 几何**
  - 位置：`desktop_workspace.dart:1022-1031`（任何 placement 事件重置 maximized + clearRestoreFrame）
  - 问题：SUPER+LMB 拖走最大化窗口后 frame 仍最大化尺寸但状态普通，restoreFrame 丢失。
  - 建议：move 类 placement 保留 maximized/restoreFrame，拖出后按 WM 惯例还原。

- [ ] **L9 — configureTracker clamp 与镜像精确策略冲突**
  - 位置：`desktop_input_layout_publisher.dart:301-306`（left/top clamp(0,16384)、w/h clamp(64,16384) 且 clamp 值存入 _configured）
  - 问题：<64px 窄窗口被 configure 成 64px（Rust 真 resizing 客户端）；与 "镜像 off-screen 动画几何" 注释冲突。
  - 建议：存原始值去重，仅发送时 clamp；或小窗口跳过 configure。

- [ ] **L10 — 协商竞态下 flag 与几何差短暂分歧**
  - 位置：`desktop_workspace.dart:1017-1020`（applyNativePlacement 用 placement 旧 flag 膨胀新几何）对照 `desktop_window_coordinator.dart:148-169`（backlog 顺序）
  - 问题：新 flag snapshot 前收到 placement 事件 → 1-2 帧内 37px 装饰与 serverSideDecorated 不匹配。
  - 建议：applyNativePlacement 读 window.serverSideDecorated（snapshot 最新值）。

- [ ] **L11 — minimized 窗口 popup 层 expects_sample=true 但场景不绘制 → buffer 钉住**
  - 位置：`desktop_input_layout_publisher.dart:161-170`（minimized 只发 mainVisibleSurfaceIds）对照 `wayland_frontend.rs:3114-3159`（expects_sample 窗口级传播到 popup 层）、`desktop_shell.dart:2281,1045`
  - 问题：popup 纹理永不被采样 → queued buffer 不 advance（最多钉 2 个，非泄漏）；恢复时先显示陈旧 popup 帧。
  - 建议：expects_sample 按 surface 级（是否在 visible_surface_ids）逐层计算而非窗口级。

- [ ] **L12 — IM popup 与主窗口潜在双重 append 隐患**
  - 位置：`wayland_frontend.rs:3131-3159`（popup 循环只取 Xdg popup）+ `3384-3524`（IM popup 独立窗口）
  - 问题：当前无重复（IM popup 不在 PopupManager）；若未来把 IM popup track_popup 进 PopupManager 会双重 append。记录防回归。
  - 建议：改动 PopupManager 时同步防护。

---

## 🔍 DPI 缩放专项（2026-08-19 第二轮审核）

> 结论：单输出任意 scale 下核心链路自洽（引擎 scale → wire → Dart 渲染 → 输入，buffer_scale/viewporter/fractional 采样无缺失）。
> gap 集中在**混合 scale 多输出**。

- [ ] **D1 — 相对指针增量用全局 atlas_scale（=max）归一**（原 L5 升级）
  - 位置：`input.rs:1924-1938`（消费于 :2586-2594）对照 `wayland_frontend.rs:940`、`topology.rs:351-356`
  - 问题：混合 scale（2x+1x）时 1x 输出上指针增量被多除 2 → 光标速度减半；`wp_relative_pointer`/拖动传给客户端的 relative delta 数值错误（客户端按 logical 理解）。
  - 严重性：**高**
  - 建议：按指针当前所在输出（pointer_location 命中的 OutputSpec.scale_120）归一；跨输出移动时切换系数。

- [ ] **D2 — 触摸板连续滚动未除 scale**
  - 位置：`input.rs:2751-2762`（`route_pointer_axis`）
  - 问题：`amount()`（libinput 物理 px）原样进 AxisFrame 发给客户端，客户端按 logical 消费 → 2x 输出上滚动速度翻倍。Flutter 路径物理直传是有意的，客户端路径必须换算。
  - 严重性：**高**
  - 建议：continuous 分量 ÷ 指针所在输出 scale；v120（鼠标滚轮）不动。

- [ ] **D3 — 绝对输入用桌面 union 逻辑 bounds 映射**
  - 位置：`input.rs:1944-1962`（绝对指针）、`2203-2218,2326-2332`（触摸）
  - 问题：`position_transformed(desktop_bounds.size)` 隐含"设备 1:1 覆盖整个桌面"假设。单输出任意 scale 下恰好正确（逻辑 bounds=物理/scale 隐含换算），但多输出时设备只覆盖一块屏 → 坐标错位；混合 scale 更甚。
  - 严重性：中
  - 建议：按设备映射的所属输出，用该输出的 logical rect（含 transform）做归一化→logical 映射。

- [ ] **D4 — scale_120 语义 = 客户端 buffer_scale×120，非输出/引擎 scale**
  - 位置：`wayland_frontend.rs:2763,2839`；`wire.rs:367,446`；`dart input_layout.dart:58`
  - 问题：buffer_scale=1 但 buffer 实为 1.5x 时 scale120=120，Dart 唯一消费点（状态栏纹理裁剪高度 `visualHeight*scale`）按 120 算 → 裁剪高度 48 而非实际 72 buffer px。渲染本身不依赖该字段不受影响，但字段名与语义易被误用。
  - 严重性：中
  - 建议：wire 改传真实 buffer/logical 比例（或 Dart 用 width/surfaceWidth 推导），加语义注释。

- [ ] **D5 — AtlasOutput.sampling 计算后从未被消费**
  - 位置：`topology.rs:376-380`
  - 问题：OneToOne/Scaled 判定全仓仅测试引用；混合 scale 时 2x 图集降采样到 1.5x/1x 全靠 KMS 硬件缩放，滤波质量取决于驱动，无法兑现 SCALING.md"downsample 保持锐利"策略。
  - 严重性：中
  - 建议：消费 sampling 选择软件滤波，或明确注释依赖硬件缩放为预期。

- [ ] **D6 — 静态纹理采样 FilterQuality.none（最近邻）**
  - 位置：`window_surface_tree.dart:12,61-134`；`desktop_shell.dart:2911`
  - 问题：2x 客户端 buffer 在 1.5x 输出降采样时摩尔纹/锯齿（与"不模糊"策略一致，但降采样质量未保证）。
  - 严重性：低
  - 建议：静态时按实际降采样比（sourceWidth/target）选 FilterQuality.medium，放大保持 none。

- [ ] **D7 — 引擎 scale 取 max 的 GPU 浪费**（设计已知）
  - 位置：`topology.rs:351-356`；SCALING.md:8-9
  - 问题：1x 输出上所有内容以 2x 光栅化再降采样回 1x —— 4 倍 GPU/内存浪费；1x 客户端 buffer 先最近邻放大 2x 再被硬件降采样，双重采样损失。
  - 严重性：低（设计已知）
  - 建议：远期支持每输出渲染或引擎 scale 跟随主输出；短期在文档量化成本。

- [ ] **D8 — scaled_edge 逐边 round 导致 1px 缝隙/重叠**
  - 位置：`topology.rs:365-374,401-403`
  - 问题：分数坐标布局（非整数 logical 边界）时相邻输出 source_rect 之间可能 1px 缝隙或重叠。
  - 严重性：低
  - 建议：共享边界计算（右边界=相邻左边界）或以整数 logical 布局。

- [ ] **D9 — native 插件窗口 scale_120 硬编码 120**
  - 位置：`native_app_plugin.rs:1408,1430`；对照 `deniald.rs:2632`
  - 问题：插件窗口描述硬编码 scale_120:120、几何=缓冲尺寸 1:1，而插件配置收到 engine_scale_120（如 240）——若插件窗口经 atlas 渲染会在 2x 引擎下放大模糊。
  - 严重性：低
  - 建议：确认插件窗口呈现路径（独立 plane 则无碍）；若走 atlas，geometry/scale 应反映引擎 scale。

- [ ] **D10 — atlas_output 逻辑原点 round() 子像素差**
  - 位置：`wayland_frontend.rs:923-937`
  - 问题：map_output 用 round 后原点，与 Flutter 场景精确 logical_origin 不一致：负坐标布局时客户端 wl_output 坐标与场景差 ≤0.5 logical px。
  - 严重性：低（子像素）
  - 建议：注释或统一为精确原点。

---

## 信息性（已评估无需修 / 已确认正确）

- `region_accepts_input` 的 `window_id == object_id` 恒真（wire 编码同源），属防御性死检查。
- fullscreen/overview 输入一致性成立：fullscreen contentRect==frame 无装饰、geometryLocked 三重锁齐备。
- 采样 ⊇ 可点集合（无 "可点但不采样" 反向缺口）。
- wire 层防御性排序（z 降序）与 publisher topmost-first 列表序一致，不会重排。
- 输出旋转对相对指针正确（atlas 逻辑布局已旋转，scanout 只做呈现变换）。
- hasSameRoutingAs 全量比较 rect/sourceRect 四值，不会漏发尺寸变化。
