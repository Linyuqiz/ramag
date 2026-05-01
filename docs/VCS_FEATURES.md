# 版本管理（VCS / Git）功能完整矩阵

> 一份"做到什么程度算实际可用"的 Git 客户端功能清单。目标对标 SourceTree / Tower / GitKraken / Fork 这一类桌面 GUI，**实用为主，用户体验第一位**。

最后更新：2026-04-30

## 优先级标记

| 标记 | 含义 | 落地版本 |
|------|------|---------|
| 🔴 **L0 必备** | 不做就完全不能用 | v0.1 |
| 🟡 **L1 应有** | 个人日常使用必需 | v0.1 - v0.2 |
| 🟢 **L2 增强** | 多分支协作必备 | v0.2 - v0.3 |
| ⚪ **L3 可选** | 高级 / 进阶场景 | 未定 / 不做 |

## 项目定位

| 维度 | 选择 | 理由 |
|------|------|------|
| 客户端形态 | macOS 原生桌面（GPUI）| 启动 <1s；万级 commit 不卡；与 dbclient 共平台一站式 |
| Git 库 | **`gix`**（gitoxide）| 纯 Rust，性能 2-10× libgit2，Zed/GitButler 都在用；API 现代 |
| 路线 | 实用 + UX 第一 | 不追完整功能集对标 DataGrip / 商业 GUI，做用户高频路径的极致体验 |
| 收费模式 | 免费 / 本地优先 | 与 ramag 整体定位一致 |

## 与同类产品对比

| 工具 | 优势 | 痛点 | ramag-vcs 对策 |
|------|------|------|----------------|
| **SourceTree** | 免费功能全 | Java 启动慢 5-10s；大仓库卡顿；UI 多年未更新；中文不友好 | Rust 启动 <1s；万 commit 流畅滚动；中文一流 |
| **GitKraken** | 分支图最美 | 商业付费；免费版限私有库；UI 偏重 | 免费 + 极简 |
| **Tower** | UX 最好 | 收费 $79/年 | 免费 |
| **Fork** | 免费够用 | 跨平台一致性弱；中文体验差 | 中文优先 |
| **IDEA Git** | commit 流程稳 | History/分支图弱；模态弹窗多 | 全部内联面板，无模态 |
| **VSCode + GitLens** | 嵌入编辑器方便 | 深度操作（rebase/冲突）弱 | 交互式 rebase + 三栏冲突 |
| **lazygit** (TUI) | 极速键盘 | 鼠标用户不友好 | 鼠标 + vim 键盘双模 |
| **GitButler** | virtual branches 创新 | 还较粗糙 | 借鉴概念，稳定实现 |

---

## 一、仓库管理（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| R01 | 添加本地仓库（指向已有目录）| 🔴 L0 | 文件选择器选 .git 父目录 |
| R02 | Clone 远程仓库 | 🔴 L0 | URL + 目标路径 + 进度条 |
| R03 | 仓库列表 / 切换 | 🔴 L0 | 类比 dbclient 连接列表 |
| R04 | 仓库收藏 / 排序 | 🟡 L1 | 常用置顶 |
| R05 | 自动识别当前 cwd 的仓库 | 🟡 L1 | 启动时探测 ramag 启动目录 |
| R06 | Init 新仓库 | 🟡 L1 | git init 流程化 |
| R07 | 仓库元数据展示 | 🟡 L1 | 大小 / commit 总数 / 分支数 |
| R08 | 仓库配置编辑 | 🟢 L2 | user.name / user.email / 别名 |
| R09 | Submodule 列表 | 🟢 L2 | 嵌套仓库识别 |
| R10 | LFS 仓库识别 | ⚪ L3 | 大文件指针 |

## 二、工作区状态（12 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| W01 | 变更文件列表（modified/added/deleted/renamed/untracked）| 🔴 L0 | 工作区核心面板 |
| W02 | 文件级 stage / unstage | 🔴 L0 | 整文件加入暂存区 |
| W03 | **Hunk 级 stage / unstage** | 🔴 L0 | 拆分提交关键能力 |
| W04 | **行级 stage / unstage** | 🟡 L1 | 比 hunk 更细 |
| W05 | Discard（丢弃工作区改动）| 🔴 L0 | 二次确认防误删 |
| W06 | 文件 diff 实时显示 | 🔴 L0 | 选中文件即右侧渲染 |
| W07 | unified / split diff 切换 | 🟡 L1 | 两种 diff 视图 |
| W08 | 二进制文件识别 + 占位 | 🟡 L1 | 不渲染 binary diff |
| W09 | 重命名检测 | 🟡 L1 | rename detection 显示 `old → new` |
| W10 | 大文件折叠提示 | 🟡 L1 | >1MB 默认折叠 |
| W11 | 文件树 / 列表两种视图 | 🟢 L2 | 嵌套深时树视图友好 |
| W12 | .gitignore 编辑助手 | 🟢 L2 | 右键 untracked 文件加入 ignore |

## 三、Commit（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| M01 | Commit message 输入 | 🔴 L0 | 多行文本框 |
| M02 | Subject + body 分离 | 🔴 L0 | 第一行 subject，空行后 body |
| M03 | Commit 提交 | 🔴 L0 | 触发底层 commit |
| M04 | Amend 上一次 | 🔴 L0 | 修改未推送的 commit |
| M05 | Commit signoff（`-s`）| 🟡 L1 | 自动加 `Signed-off-by:` |
| M06 | 自动 lint（subject 长度 / blank line）| 🟡 L1 | 提示但不强制 |
| M07 | Commit message 模板 | 🟡 L1 | `commit.template` 配合 |
| M08 | **AI 生成 commit message** | 🟢 L2 | 选中 staged hunks → Claude 生成 |
| M09 | GPG / SSH 签名 | 🟢 L2 | 配置项 + 签名校验 |
| M10 | Commit message 历史回查 | ⚪ L3 | 上次写过的快速选 |

## 四、分支（12 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| B01 | 本地分支列表 | 🔴 L0 | 当前分支高亮 |
| B02 | 远程分支列表 | 🔴 L0 | 按 remote 分组 |
| B03 | 切换分支（checkout）| 🔴 L0 | 工作区有改动时提示 |
| B04 | 创建分支 | 🔴 L0 | 基于当前 HEAD / 选 commit |
| B05 | 删除分支 | 🔴 L0 | 防止删未合并的（force 二次确认）|
| B06 | 重命名分支 | 🟡 L1 | 含远程同步选项 |
| B07 | 跟踪远程分支（set-upstream）| 🟡 L1 | push -u 等价 |
| B08 | Merge 分支 | 🔴 L0 | 进当前分支 |
| B09 | Rebase 分支 | 🔴 L0 | 基础 rebase（非交互式）|
| B10 | **交互式 Rebase**（drag-drop）| 🟢 L2 | squash/fixup/edit/drop |
| B11 | Cherry-pick | 🟡 L1 | 单个 / 批量 |
| B12 | Reset（soft / mixed / hard）| 🟡 L1 | 带预览影响 |

## 五、History / Log（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| H01 | 提交日志列表 | 🔴 L0 | hash / author / date / subject |
| H02 | 流式 / 分页加载 | 🔴 L0 | 万 commit 仓库不卡 |
| H03 | **可视化分支图** | 🟡 L1 | Canvas 节点 + 边，颜色编码 |
| H04 | 提交详情面板 | 🔴 L0 | 选中 commit → 文件变更列表 + diff |
| H05 | 单文件历史 | 🟡 L1 | `git log --follow <file>` |
| H06 | Search commits | 🟡 L1 | hash / message / author / 文件路径 |
| H07 | 时间范围过滤 | 🟢 L2 | since / until |
| H08 | 分支 / 作者过滤 | 🟢 L2 | 多 tag |
| H09 | **Blame view** | 🟡 L1 | 行级追溯，hover 弹 commit |
| H10 | Reflog | 🟢 L2 | 误操作恢复 |

## 六、远程同步（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| S01 | Push（带进度）| 🔴 L0 | objects/速率/ETA 而不是 spinner |
| S02 | Pull（fetch + merge）| 🔴 L0 | 默认 fast-forward |
| S03 | Pull --rebase | 🔴 L0 | 配置项 |
| S04 | Fetch all | 🔴 L0 | 拉远程更新但不合并 |
| S05 | Push tags | 🟡 L1 | 单独 / 一并 |
| S06 | Push --force-with-lease | 🟡 L1 | 安全的 force push |
| S07 | Remote 管理（add/remove/rename/set-url）| 🟡 L1 | 远程 CRUD |
| S08 | HTTPS / SSH 凭证 | 🔴 L0 | 钥匙串集成 |
| S09 | 多 remote（origin + upstream）| 🟡 L1 | fork 流程 |
| S10 | Git Credential Manager 集成 | 🟢 L2 | 复用 git credential |

## 七、Tag（5 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| T01 | Tag 列表 | 🟡 L1 | lightweight + annotated |
| T02 | 创建 Tag | 🟡 L1 | 基于当前 / 选 commit |
| T03 | 删除 Tag | 🟡 L1 | 含远程同步选项 |
| T04 | 推送 Tag | 🟡 L1 | 单个 / 全部 |
| T05 | 签名 Tag | ⚪ L3 | GPG annotated |

## 八、Stash（6 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| K01 | Stash 列表 | 🔴 L0 | **像分支一样独立面板**，不藏菜单 |
| K02 | Stash 创建 | 🔴 L0 | 含可选 message + untracked 选项 |
| K03 | Apply / Pop | 🔴 L0 | 区分应用与弹出 |
| K04 | Drop | 🔴 L0 | 二次确认 |
| K05 | Stash diff 查看 | 🟡 L1 | 选中即右侧显 diff |
| K06 | 部分 stash（hunk 级）| 🟢 L2 | git stash push --patch |

## 九、Conflict 解决（5 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|--------|------|
| F01 | Conflict 状态识别 | 🔴 L0 | merge / rebase 进行中提示 |
| F02 | **三栏视图**（ours / theirs / result）| 🟡 L1 | 逐 hunk 选择来源 |
| F03 | 一键 take ours / take theirs | 🟡 L1 | 整文件粒度 |
| F04 | 冲突标记跳转（`<<<<<<<`）| 🟡 L1 | 列表 + 跳转 |
| F05 | 外部 mergetool 集成 | 🟢 L2 | 调用配置好的 vimdiff/kaleidoscope |

## 十、UX 创新点（区别于其他 GUI）

| ID | 体验 | 优先级 | 说明 |
|----|------|--------|------|
| U01 | **零模态**：所有操作内联面板完成 | 🔴 L0 | commit / merge / push 不弹中间窗 |
| U02 | **命令面板 ⌘K** | 🟡 L1 | 搜索式 git 操作入口（"stage all" / "checkout main"）|
| U03 | **Vim 键盘驱动** | 🟡 L1 | `j/k` 行间，`s` stage，`u` unstage，`c` commit，`/` 搜索 |
| U04 | **inline diff 默认** | 🔴 L0 | 选文件即实时显，不点不弹 |
| U05 | **未推送强化提示** | 🔴 L0 | Push 按钮带 N badge，分支图未推送节点描灰边 |
| U06 | **进度细节** | 🔴 L0 | "objects 234/1000 · 1.2 MB/s · ETA 3s"，不是 spinner |
| U07 | **Drag drop**：拖文件 stage / 拖 commit cherry-pick | 🟢 L2 | 直觉操作 |
| U08 | **大仓库流式加载** | 🔴 L0 | uniform_list 虚拟滚动，复用 dbclient 模式 |
| U09 | **AI commit message**（Claude）| 🟢 L2 | 可选 feature flag |
| U10 | **stash 像分支**：独立面板 | 🔴 L0 | 不藏菜单 |
| U11 | **DB + Git 一站式** | 🔴 L0 | 同窗口 tab 切换：dbclient ↔ vcs |

## 十一、明确不做（避免内耗）

| 项 | 理由 |
|----|------|
| 完全 git CLI 等价 GUI | git 命令上千个，对标 SourceTree/Tower 即可（覆盖 95% 日常）|
| Web 界面 / 远程托管 | 那是 Gitea / Forgejo 的事 |
| Issue / PR 管理 | GitHub / GitLab CLI 已足够；本工具聚焦本地 |
| Git 教学引导 | 假设用户已经懂 git 基本概念 |
| 自动 push（commit 即推）| 安全反模式，不做 |

---

## 架构（看代码前必读）

跟 dbclient 完全对称的分层：

```
ramag-bin                ← 启动时多注册一个 VcsTool
  ├── ramag-tool-vcs           ← VCS 主视图（仓库列表 / 工作区 / commit / history / 分支等）
  ├── ramag-infra-git          ← impl GitDriver trait（用 gix）
  └── ramag-app                ← 加 VcsService（用例聚合，参考 ConnectionService 模式）
        └── ramag-domain       ← entities/git_*.rs + traits/git_driver.rs
```

### 核心 trait

`GitDriver`（在 `ramag-domain/src/traits/git_driver.rs`）：

```rust
#[async_trait]
pub trait GitDriver: Send + Sync {
    async fn open_repo(&self, path: &Path) -> Result<RepoHandle>;
    async fn status(&self, repo: &RepoId) -> Result<WorkingTreeStatus>;
    async fn stage(&self, repo: &RepoId, paths: &[String]) -> Result<()>;
    async fn unstage(&self, repo: &RepoId, paths: &[String]) -> Result<()>;
    async fn discard(&self, repo: &RepoId, paths: &[String]) -> Result<()>;
    async fn commit(&self, repo: &RepoId, message: &str, amend: bool) -> Result<CommitId>;
    async fn list_branches(&self, repo: &RepoId, kind: BranchKind) -> Result<Vec<Branch>>;
    async fn checkout(&self, repo: &RepoId, target: &str) -> Result<()>;
    async fn create_branch(&self, repo: &RepoId, name: &str, base: Option<&str>) -> Result<()>;
    async fn delete_branch(&self, repo: &RepoId, name: &str, force: bool) -> Result<()>;
    async fn log(&self, repo: &RepoId, opts: LogOptions) -> Result<Vec<Commit>>;
    async fn diff_file(&self, repo: &RepoId, path: &str, kind: DiffKind) -> Result<FileDiff>;
    async fn fetch(&self, repo: &RepoId, remote: &str) -> Result<()>;
    async fn push(&self, repo: &RepoId, remote: &str, branch: &str) -> Result<()>;
    async fn pull(&self, repo: &RepoId, remote: &str, branch: &str, rebase: bool) -> Result<()>;
    async fn list_stashes(&self, repo: &RepoId) -> Result<Vec<Stash>>;
    async fn stash_save(&self, repo: &RepoId, message: Option<&str>, include_untracked: bool) -> Result<()>;
    async fn stash_apply(&self, repo: &RepoId, idx: usize, pop: bool) -> Result<()>;
    async fn stash_drop(&self, repo: &RepoId, idx: usize) -> Result<()>;
    // 后续阶段补充：tag / cherry-pick / merge / rebase / blame / reflog ...
}
```

### 核心实体

- `repo.rs`：`RepoId` / `RepoHandle` / `RepoConfig`
- `status.rs`：`WorkingTreeStatus` / `FileStatus` / `FileChange`
- `commit.rs`：`Commit` / `CommitId` / `CommitMeta`（author/committer/timestamps）
- `branch.rs`：`Branch` / `BranchKind`（Local/Remote）
- `diff.rs`：`FileDiff` / `Hunk` / `DiffLine` / `DiffKind`（WorkingTree/Staged/Commit）
- `stash.rs`：`Stash` / `StashId`
- `remote.rs`：`Remote` / `RemoteUrl`
- `tag.rs`：`Tag` / `TagKind`

### 技术决策

#### 1) 为什么用 gix 不用 git2

| 项 | gix | git2 (libgit2) |
|----|-----|----------------|
| 实现 | 纯 Rust | C 库 binding |
| 性能 | 2-10× 快 | 基线 |
| API | 现代 / async-friendly | C 风格，需 wrapper |
| 维护 | 活跃（GitHub 万星）| 慢，部分新特性缺失 |
| 已用于 | Zed / GitButler / cargo | GitKraken 早期等 |
| 缺点 | 部分功能尚缺（rebase 复杂场景）| 无 |

ramag 选 gix。短期缺的功能（如交互式 rebase 内部细节）通过 fallback 调 `git` 命令兜底。

#### 2) 异步桥接

gix 主要是同步 API（部分异步），跟 sqlx 全 async 风格不同。沿用 ramag 现有桥接模式：
- `ramag-infra-git/src/runtime.rs`：`std::thread + futures::oneshot` 把同步 gix 调用转成 GPUI 友好的 async future（参考 `ramag-infra-storage` 同款桥接）
- 不需要 tokio runtime（gix 不依赖 tokio）

#### 3) 大仓库性能

| 维度 | 策略 |
|------|------|
| log 列表 | 流式迭代器 → 上层分页（每屏 100 条），uniform_list 虚拟滚动 |
| diff | 单文件 lazy 计算，只在用户点选时跑 |
| status | 增量 watch（fs notify）→ 仅扫变化文件 |
| 分支图 | 客户端绘制时只算视口内 commits，下滑时增量算 |

#### 4) 凭证存储

复用现有 `ramag-infra-storage` + macOS 钥匙串：
- HTTPS user/pwd 走钥匙串
- SSH key 走系统 SSH agent
- 不持久化任何明文凭证

---

## 实施路线图

### Phase A：骨架（1 周）

| 任务 | 输出 |
|------|------|
| domain 加 git 实体 + GitDriver trait | `entities/git_*.rs` + `traits/git_driver.rs` |
| 新建 `ramag-infra-git` crate（gix 桥接 + 空实现）| trait 实现框架 |
| 新建 `ramag-tool-vcs` crate（空 view + Tool trait）| 空 view 显示「VCS 即将上线」 |
| `ramag-app` 加 `VcsService` | use case 聚合 |
| `ramag-bin` 注册 VcsTool | 首页"版本管理"卡片可点 |
| 主页 home_view 把 soon_module_card 升级 active_module_card | 入口可达 |

### Phase B：仓库 + 状态 daily flow（1.5 周）

| 任务 | 完成标准 |
|------|----------|
| 仓库选择 / 添加 | 可打开本地任意 git 仓库 |
| 工作区 status 面板 | 列出变更文件 + 类型标识 |
| 文件 / hunk 级 stage/unstage/discard | 可正常进入 staged 区 |
| Commit 面板 | 输入 message → 提交成功 → 状态刷新 |
| Diff 渲染（unified）| 选中文件右侧实时显 diff |

### Phase C：分支 + 远程（1 周）

| 任务 | 完成标准 |
|------|----------|
| 本地 / 远程分支列表 | 当前分支高亮，可双击切换 |
| 创建 / 删除分支 | 含确认 |
| Fetch / Push / Pull（带进度）| 不再 spinner，用 objects/speed/ETA |
| Stash 列表 + apply/drop | 独立面板 |

### Phase D：History + 打磨（1 周）

| 任务 | 完成标准 |
|------|----------|
| History 列表 + 流式加载 | 万 commit 不卡 |
| 选中 commit 看变更 + diff | 复用 diff_view |
| ⌘K 命令面板 | 全 git 操作搜索式入口 |
| Vim 键盘驱动 | `j/k/s/u/c/?` |
| 键盘快捷键 + tooltip 完善 | 全面板可达 |

**v0.1 总工期：4-5 周**。验收标准：能完整替代 SourceTree 80% 日常操作。

### Phase E（v0.2 起）：分支图 + 高级

- 可视化分支图（Canvas）
- Cherry-pick / Reset / Tag
- Conflict 三栏解决器
- Blame view + 文件历史
- Search commits

### Phase F（v0.3 起）：高级 + 创新

- 交互式 Rebase（drag drop）
- AI commit message
- Submodule / Worktree / LFS
- Bisect 向导

---

## 编程规范

跟 dbclient 同款：

- `ramag-domain` 不依赖 git2 / gix，仅持 trait + 实体
- `ramag-infra-git` 不依赖 GPUI
- `ramag-tool-vcs` 单文件 ≤300 行，绝对不超 600 行
- 中文注释 + 英文 grep-friendly 日志
- Action 加 `#[action(namespace = ramag_vcs)]` 命名空间隔离
