# 架构

本文档描述 TermIDE 的技术架构。

## 概述

TermIDE 是一个使用 Rust 语言基于 `ratatui` TUI 框架构建的基于终端的 IDE。它采用创新的**手风琴面板布局系统**，可自适应终端宽度，实现高效的多面板工作流。

```
┌─────────────────────────────────────────────────────────┐
│ 菜单栏     [CPU] [RAM] [时钟]                            │
├───────────────────┬─────────────────────────────────────┤
│ ┌[≡][📁] 文件 ──┐ │ ┌[≡][📝] 编辑器: main.rs ──────────┐│
│ │               │ │ │                                  ││
│ │ src/          │ │ │  fn main() {                     ││
│ │ tests/        │ │ │      // code here                ││
│ │ Cargo.toml    │ │ │  }                               ││
│ │               │ │ │                                  ││
│ └───────────────┘ │ └──────────────────────────────────┘│
│ ─[≡][💻] 终端─── │ ─[≡][📋] 日志 ──────────────────────│
├───────────────────┴─────────────────────────────────────┤
│ 状态: file.rs:42  行 10, 列 5        磁盘: 83%          │
└─────────────────────────────────────────────────────────┘
```

## 核心架构组件

### 1. 布局系统

#### 1.1 LayoutManager

**位置：** `crates/layout/src/lib.rs`

`LayoutManager` 是手风琴布局系统的核心。它管理：

**组成部分：**
- `panel_groups: Vec<PanelGroup>` - 面板组的水平排列
- `focus: usize` - 当前焦点（活动面板组的索引）

**主要职责：**
- 基于宽度阈值自动堆叠添加面板
- 管理水平导航（Alt+Left/Right）
- 管理组内垂直导航（Alt+Up/Down）
- 智能面板堆叠/拆分（Alt+Backspace）
- 关闭面板并清理空组

**焦点管理：**
焦点是一个简单的 `usize` 索引，指示当前活动的面板组。获得焦点的组接收键盘/鼠标输入，并在 UI 中高亮显示。

#### 1.2 PanelGroup

**位置：** `crates/layout/src/panel_group.rs`

`PanelGroup` 表示具有手风琴行为的面板垂直堆叠。

**结构：**
```rust
pub struct PanelGroup {
    panels: Vec<Box<dyn Panel>>,  // 此组中的面板
    expanded_index: usize,         // 哪个面板是展开的
    pub width: Option<u16>,        // 字符宽度（None = 自动分配）
}
```

**手风琴行为：**
- 恰好一个面板处于展开状态（显示完整内容）
- 其他面板折叠为仅显示标题栏
- 点击标题栏面板图标按钮进行展开/折叠
- Alt+Up/Down 在组内面板之间导航

**关键操作：**
- `add_panel()` - 向组中添加面板
- `remove_panel()` - 移除面板（如需重置 expanded_index）
- `set_expanded()` - 更改展开的面板
- `next_panel()` / `prev_panel()` - 循环切换面板

#### 1.3 自动堆叠

通过 `LayoutManager::add_panel()` 添加新面板时：

```rust
let new_width_if_split = available_width / (num_groups + 1);

if new_width_if_split < config.min_panel_width {
    // 在当前组中垂直堆叠（手风琴式）
    active_group.add_panel(panel);
} else {
    // 创建新的水平组
    let new_group = PanelGroup::new(panel);
    panel_groups.push(new_group);
}
```

**默认阈值：** `min_panel_width = 80` 字符

这确保面板始终有足够的空间保持可用性。

### 2. 面板系统

#### 2.1 Panel Trait

**位置：** `crates/core/src/lib.rs`

所有面板实现 `Panel` trait，该 trait 定义了交互式终端面板的接口：

```rust
pub trait Panel {
    /// 渲染面板内容
    fn render(
        &mut self,
        area: Rect,                // 可用渲染区域
        buf: &mut Buffer,          // Ratatui 缓冲区
        is_focused: bool,          // 此面板是否有焦点？
        panel_index: usize,        // 面板索引用于标识
        state: &AppState,          // 共享应用状态
    );

    /// 处理键盘输入
    fn handle_key(&mut self, key: KeyEvent) -> Result<()>;

    /// 处理鼠标输入
    fn handle_mouse(&mut self, mouse: MouseEvent, panel_area: Rect) -> Result<()>;

    /// 获取面板标题（显示在标题栏中）
    fn title(&self) -> String;

    /// 检查是否为欢迎面板（打开其他面板时自动关闭）
    fn is_welcome_panel(&self) -> bool { false }

    /// 获取要打开的文件（用于请求打开文件的面板）
    fn take_file_to_open(&mut self) -> Option<PathBuf> { None }

    /// 获取新面板的工作目录
    fn get_working_directory(&self) -> Option<PathBuf> { None }

    /// 获取模态框请求（用于打开模态框的面板）
    fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> { None }
}
```

#### 2.2 面板实现

**文件管理器** (`crates/panel-file-manager/src/lib.rs`)
- 浏览文件和目录
- 文件操作（创建、删除、复制、移动）
- Git 状态集成
- 剪贴板支持
- 批量操作
- 拖放选择

**编辑器** (`crates/panel-editor/src/lib.rs`)
- 带撤销/重做的文本编辑
- 通过 tree-sitter 实现语法高亮（15+ 种语言）
- 带交互式模态框的搜索和替换
- 行号、光标位置、自动换行
- 单词导航（Ctrl+Left/Right）、段落/符号导航（Ctrl+Up/Down）
- 带括号分割缩进的自动缩进
- 自动关闭括号和引号
- 行号中的 Git 差异可视化
- LSP 集成（补全、悬停、跳转到定义）
- 文件保存，支持另存为和可执行复选框

**终端** (`crates/panel-terminal/src/lib.rs`)
- 完整的 PTY（伪终端）支持
- Shell 集成
- 回滚缓冲区
- 跨滚动缓冲区和可见缓冲区的文本搜索（`Searchable` trait）
- ANSI 颜色支持
- 调整大小处理

**日志查看器** (`crates/panel-misc/src/journal.rs`)
- 应用日志查看器
- 面板信息
- 系统资源监控

**Git 状态** (`crates/panel-git-status/src/lib.rs`)
- 仓库状态概览
- 文件暂存/取消暂存
- 分支切换
- 创建提交

**Git 日志** (`crates/panel-git-log/src/lib.rs`)
- 带 ASCII 图形的提交历史
- 查看差异
- 复制提交哈希

**Git 差异** (`crates/panel-git-diff/src/lib.rs`)
- 并排或内联差异视图
- 语法高亮的差异

**诊断** (`crates/panel-diagnostics/src/lib.rs`)
- LSP 诊断显示
- 错误/警告导航

**操作** (`crates/panel-operations/src/lib.rs`)
- 后台文件操作跟踪
- 复制/移动/删除的进度显示

**大纲** (`crates/panel-outline/src/lib.rs`)
- 基于 tree-sitter 查询的代码结构导航
- 与活动编辑器同步的符号列表
- 按 Enter 导航到符号
- 光标跟踪和实时更新

**图片** (`crates/panel-image/src/lib.rs`)
- 原生图片渲染（Kitty、iTerm2、Sixel 协议）
- 回退到 Unicode 块字符

**帮助** (`crates/panel-misc/src/help.rs`)
- 基于快捷键配置动态生成的帮助内容
- 带全宽布局的伪图形表格
- 支持键盘和鼠标滚动
- 打开其他面板时自动关闭

### 3. 事件处理

#### 3.1 事件循环

**位置：** `crates/app/src/app/mod.rs`

主事件循环结构：

```rust
while !state.should_quit {
    match event_handler.next()? {
        Event::Key(key) => self.handle_key_event(key)?,
        Event::Mouse(mouse) => self.handle_mouse_event(mouse)?,
        Event::Resize(w, h) => state.update_terminal_size(w, h),
        Event::Tick => {
            // 周期性更新
            self.update_panels_tick()?;
            self.system_monitor.update(&mut self.state);
        }
    }
    self.render(terminal)?;
}
```

**事件类型：**
- **Key** - 键盘输入（快捷键、文字输入）
- **Mouse** - 鼠标点击、拖拽、滚动
- **Resize** - 终端大小变化
- **Tick** - 周期性定时器（资源监控、面板更新）

#### 3.2 键盘处理器

**位置：** `crates/app/src/app/key_handler.rs`

按优先级处理键盘输入：

1. **模态框优先捕获输入**（如果已打开）
2. **全局快捷键**（Alt+M、Alt+H、Alt+Q 等）
3. **面板管理**（Alt+Left/Right、Alt+Up/Down、Alt+X 等）
4. **活动面板**（通过 `panel.handle_key()`）

**西里尔文支持：**
通过 `termide_keyboard::translate_hotkey()` 进行键盘布局翻译，使快捷键在俄语键盘布局下也能工作。

#### 3.3 鼠标处理器

**位置：** `crates/app/src/app/mouse_handler.rs`

处理鼠标输入：

**面板标题栏：**
- 点击 `[≡]` 按钮 → 打开面板操作上下文菜单（关闭 / 拆分 / 合并 / 移动）
- 点击 `[▶]/[▼]` 按钮 → 切换展开/折叠
- 点击标题区域 → 激活面板（双击文件管理器 → 目录选择器）
- **拖拽标题区域** → 面板 drag-and-drop：ghost 跟随光标，drop 区被高亮；释放在另一个面板的标题上则插入到该组，释放在组之间则创建新组。`Escape` 取消。

**面板内容：**
- 点击转发到 `panel.handle_mouse()`
- 每个面板处理自己的鼠标交互

**菜单栏：**
- 点击菜单项进行激活

#### 3.4 模态框处理器

**位置：** `crates/app/src/app/modal_handler.rs` 和 `crates/modal/src/`

处理交互式模态对话框：

**模态框类型**（crate `termide-modal`）：
- **Input** — 文本输入（文件名、目录名等）
- **Confirm** — 是/否确认
- **Select** / **EditableSelect** — 从选项中选择（可编辑）
- **Choice** — 水平选择按钮
- **Info** — 信息显示，**支持内容滚动**（脚本报告、系统信息）；右侧边框带滚动条，支持 `↑↓/PageUp/PageDown/Home/End` 及鼠标滚轮
- **InfoAction** — 带附加操作按钮的信息窗口
- **Settings** — 采用 **侧边栏布局** 的全屏配置模态。拆分为 `crates/modal/src/settings/` 下的若干子模块：
  - `settings.rs` — `SettingsModal` 结构、渲染、键盘/鼠标处理
  - `settings/fields.rs` — 声明性字段数据（`FieldType`、`FieldDescriptor`、`ContentRow`，以及辅助函数 `fields_for_tab`、`get_field_value`、`toggle_field`、`cycle_enum_*`）
  - `settings/kb.rs` — 键绑定表和宏（`kb_get!`/`kb_set!`、`KB_SECTIONS`、`kb_binding_names`、`get/set_kb_value`、`format_key_event`）
- **Progress** — 长时间操作的进度条
- **Commit** / **Conflict** / **RenamePattern** / **Search** / **Replace** / **TreeSearch** / **Sessions** / **DirectoryPicker** / **SaveAs** / **BookmarkAdd** / **Calendar** / **CommandPalette** / **ScriptCreate** — 针对具体场景的专用对话框

共用工具集中在 `crates/modal/src/base.rs`（`render_modal_block`、`render_modal_frame`、`button_style`、`CursorNavigation` trait）。

**输入捕获：**
模态框打开时，键盘输入首先传递给模态框。Escape 关闭模态框。

### 4. 渲染管线

#### 4.1 主渲染

**位置：** `crates/ui-render/src/layout.rs`

渲染流程：

```rust
fn render_layout_with_accordion(frame, layout_manager, state) {
    // 1. 计算所有面板组的水平布局
    let horizontal_chunks = calculate_horizontal_layout();

    // 2. 渲染面板组
    for group in groups {
        let vertical_chunks = calculate_vertical_layout(group);

        // 3. 渲染每个面板（展开或折叠）
        for panel in group {
            if is_expanded {
                render_expanded_panel(panel, area, ...);
            } else {
                render_collapsed_panel(panel, area, ...);
            }
        }
    }

    // 4. 渲染模态框（如果打开）
    if let Some(modal) = state.active_modal {
        render_modal(modal, ...);
    }
}
```

#### 4.2 面板渲染

**位置：** `crates/ui-render/src/panel.rs`

**展开的面板：**
- 带 `[≡][图标]` 按钮和标题的边框（例如 `[≡][📁] 文件`）
- 完整的内容区域
- 内容超出区域时可滚动

**折叠的面板：**
- 仅标题栏：`─[≡][📁] 文件 ─────`
- 占用最小垂直空间（1 行）
- 点击即展开

**图标模式：**
面板标题根据面板类型显示 emoji 图标（📁 文件管理器、💻 终端、📝 编辑器等）。通过 `[general]` 中的 `icon_mode` 配置：
- `auto`（默认）— 终端支持时显示 emoji，否则仅显示 `[≡]`
- `emoji` — 始终显示 emoji 图标
- `unicode` — 无图标、无箭头，仅 `[≡]`

**Drag Overlay：**
拖拽面板顶部边框时，`render_drag_overlay()`（在 `src/ui.rs`）在主面板渲染之后、dropdowns/modals 之前运行。它高亮 drop 目标（`IntoGroup` 是目标面板的顶部边框，`NewGroup` 是组间的垂直线）并在光标下绘制 ghost 图标。命中测试重用 `termide_layout` 中的自由函数 `calculate_panel_rects` / `compute_drop_target`，因此鼠标处理器和渲染器对几何一致。

**边框渲染：**
边框和按钮由 `panel_rendering.rs` 绘制，然后面板的 `render()` 方法在内部区域绘制内容。

### 5. 状态管理

#### 5.1 AppState

**位置：** `crates/state/src/`（拆分为 `batch.rs`、`layout.rs`、`operations.rs`、`pending_action.rs`、`ui.rs`）

中央状态容器：

```rust
pub struct AppState {
    pub theme: Theme,                    // 当前主题
    pub terminal: TerminalInfo,          // 宽度、高度
    pub config: Config,                  // 用户配置
    pub should_quit: bool,               // 退出标志
    pub batch_operation: Option<BatchOp>, // 待处理的批量操作
    pub active_modal: Option<ActiveModal>, // 当前模态框
    pub error_message: Option<String>,   // 要显示的错误
    pub fs_watcher: Option<Watcher>,     // 文件系统监视器
    // ... 其他字段
}
```

**线程安全：**
大部分状态为单线程（TUI 在主线程运行）。文件系统监视器使用通道进行跨线程通信。

#### 5.2 配置

**位置：** `crates/config/src/lib.rs`

从 TOML 加载的用户配置：

```rust
pub struct Config {
    pub general: GeneralSettings,         // 主题、语言、icon_mode、vim_mode、快捷键
    pub editor: EditorSettings,           // 制表符大小、自动换行、git diff、自动缩进
    pub file_manager: FileManagerSettings, // 扩展视图宽度、快捷键
    pub git_status: GitStatusSettings,    // 快捷键
    pub terminal: TerminalSettings,       // 快捷键
    pub lsp: LspSettings,                // LSP 服务器、补全、悬停
    pub logging: LoggingSettings,         // 日志级别、资源监控间隔
    pub vfs: VfsSettings,                // VFS 连接超时
}
```

**默认位置：**
- Linux: `~/.config/termide/config.toml`
- macOS: `~/Library/Application Support/termide/config.toml`
- Windows: `%APPDATA%\\termide\\config.toml`

### 6. 主题系统

**位置：** `crates/theme/src/lib.rs`

**内置主题：** 38 款主题（Dracula、Nord、Monokai、Matrix、Pip-Boy 等）

**自定义主题：** 从 `~/.config/termide/themes/*.toml` 加载

**主题结构：**
```rust
pub struct Theme {
    pub fg: Color,                // 前景色
    pub bg: Color,                // 背景色
    pub accented_fg: Color,       // 聚焦元素
    pub disabled: Color,          // 禁用/非聚焦
    pub selected_bg: Color,       // 选择背景
    // ... 语法高亮颜色
}
```

**加载优先级：**
1. 用户主题（在配置目录中）
2. 内置主题
3. 回退到默认

### 7. 国际化

**位置：** `crates/i18n/`

通过编译时加载的基于 TOML 的翻译文件实现语言支持：

```
crates/i18n/
├── src/
│   ├── lib.rs      # 翻译 trait 和运行时
│   └── runtime.rs  # 语言检测和加载
└── i18n/           # 翻译文件
    ├── bn.toml     # 孟加拉语
    ├── de.toml     # 德语
    ├── en.toml     # 英语
    ├── es.toml     # 西班牙语
    ├── fr.toml     # 法语
    ├── hi.toml     # 印地语
    ├── id.toml     # 印尼语
    ├── ja.toml     # 日语
    ├── ko.toml     # 韩语
    ├── pt.toml     # 葡萄牙语
    ├── ru.toml     # 俄语
    ├── th.toml     # 泰语
    ├── tr.toml     # 土耳其语
    ├── vi.toml     # 越南语
    └── zh.toml     # 中文
```

**语言：** 支持 15 种（孟加拉语、中文、英语、法语、德语、印地语、印尼语、日语、韩语、葡萄牙语、俄语、西班牙语、泰语、土耳其语、越南语）

**检测顺序：**
1. `config.language` 设置
2. `LANG` / `LC_ALL` 系统变量
3. 默认为英语

### 8. 关键依赖

**Ratatui** - 终端 UI 框架
- 基于组件的渲染
- 缓冲区系统实现高效更新
- 布局系统（Rect、Constraints）

**Crossterm** - 跨平台终端控制
- 事件处理（键盘、鼠标、调整大小）
- 终端控制（光标、颜色、清屏）
- Raw 模式管理

**Tree-sitter** - 语法高亮
- 15+ 种语言的解析器生成器
- 增量解析提升性能
- 用于语法高亮的查询系统

**Ropey** - 文本缓冲区
- 高效的基于行的文本存储
- UTF-8 感知
- 内部采用 Gap buffer

**Portable-pty** - PTY 实现
- 跨平台伪终端
- Shell 集成
- 调整大小支持

**Sysinfo** - 系统监控
- CPU 使用率
- 内存使用量
- 磁盘空间

## 设计决策

### 为什么选择手风琴布局？

**问题：** 终端空间有限，多面板 IDE 常常显得拥挤。

**解决方案：** 手风琴布局最大化可用空间：
- 每组一个展开的面板获得完整的垂直空间
- 其他面板折叠为标题栏（1 行）
- 通过 Alt+Up/Down 或鼠标点击快速访问
- 终端太窄时自动堆叠

### 为什么选择动态面板？

**优势：** 用户可以根据需要打开任意数量的面板：
- 多个编辑器用于不同文件
- 多个终端用于不同任务
- 多个文件管理器用于不同目录

**挑战：** 高效管理多个面板
- 手风琴防止混乱
- 快捷键提供快速导航
- 欢迎界面自动关闭

### 为什么选择基于 Trait 的面板？

**灵活性：** 无需更改核心代码即可添加新面板类型
- 实现 `Panel` trait
- 添加到面板创建逻辑
- 与现有布局系统协同工作

**多态性：** `Box<dyn Panel>` 允许异构集合
- 单个 `Vec<Box<dyn Panel>>` 容纳所有面板类型
- 统一的渲染和事件处理
- 动态分发的开销对 TUI 来说可忽略不计

## 性能特征

**渲染：** O(n)，n = 可见面板数量
- 仅展开的面板渲染完整内容
- 折叠的面板渲染单行

**事件处理：** 大多数操作 O(1)
- 直接索引访问聚焦面板
- 快捷键绑定使用哈希表查找

**内存：** 与面板数量线性相关
- 每个面板拥有自己的状态
- 共享的 AppState 很小
- 无过度克隆（使用引用）

**文件操作：** 尽可能异步
- 文件系统监视器使用独立线程
- 防抖避免过多更新

### 8. 会话管理

**位置：** `crates/session/src/lib.rs`

会话持久化允许保存和恢复面板布局：

**存储位置：**
- Linux: `~/.local/share/termide/sessions/<project_path>/session.toml`
- macOS: `~/Library/Application Support/termide/sessions/<project_path>/session.toml`

**功能特性：**
- 退出时自动保存会话
- 启动时恢复面板布局
- 通过菜单切换会话（在不同项目之间切换）
- 会话保留，自动清理旧会话

**会话文件格式：**
```toml
[[panel_groups]]
expanded_index = 0
horizontal_weight = 100

[[panel_groups.panels]]
panel_type = "file_manager"
state = { current_path = "/home/user/project" }

[[panel_groups.panels]]
panel_type = "editor"
state = { file_path = "/home/user/project/main.rs", cursor_line = 42 }
```

## 未来架构考虑

**潜在改进：**

1. **异步面板**
   - 长时间运行的操作（搜索、编译）不阻塞 UI
   - 带进度指示器的后台任务

2. **插件系统**
   - 动态加载面板
   - 用户定义的面板类型
   - 脚本集成（Lua、Python）

3. **网络面板**
   - SSH 终端面板
   - 远程文件浏览器
   - 协作编辑

## 调试架构

**日志系统：**
- 所有日志写入配置目录中的 `termide.log`
- 级别：INFO、ERROR、DEBUG
- 时间戳和组件前缀
- 日志轮转防止无限增长

**调试面板：**
- 应用状态实时查看
- 最近的日志条目
- 面板检查
- 性能指标

**Panic 处理：**
- Panic 时恢复终端
- 将 panic 信息写入日志
- 向用户显示错误消息

## 安全考虑

**终端注入：**
- 终端面板中过滤 ANSI 转义序列
- Shell 执行前对用户输入进行清理

**文件操作：**
- 防止符号链接攻击
- 路径遍历检查
- 操作前进行权限检查

**资源限制：**
- 编辑器文件大小限制（100 MB）
- 终端回滚缓冲区限制
- 日志轮转防止磁盘耗尽

## 总结

TermIDE 的架构优先考虑：
- **灵活性** - 动态面板系统适应用户需求
- **高效性** - 手风琴布局最大化可用空间
- **可扩展性** - 基于 Trait 的设计便于扩展
- **健壮性** - 防御性编程防止崩溃
- **性能** - 高效的渲染和事件处理

手风琴布局系统是 TermIDE 区别于传统多面板终端应用程序的关键创新。
