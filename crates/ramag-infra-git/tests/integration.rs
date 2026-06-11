//! 集成测试：对真实临时 git 仓库跑端到端操作，验证 Git 功能真实可用。
//! git 是本地命令，无需环境变量；缺 git 时 setup 会 panic。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use futures::executor::block_on;
use ramag_domain::entities::{
    BranchKind, CommitId, DiffKind, DiffLineKind, FileChangeKind, LogOptions, RepoId, ResetKind,
};
use ramag_domain::traits::GitDriver;
use ramag_infra_git::GitDriverImpl;

/// 设 git 仓库级配置（commit 需要 user.name/email）
fn git_config(dir: &Path, key: &str, val: &str) {
    let ok = std::process::Command::new("git")
        .args(["-C", dir.to_str().unwrap(), "config", key, val])
        .status()
        .unwrap()
        .success();
    assert!(ok, "git config {key} 失败");
}

/// 建临时仓库 + 配置 user，返回 (driver, repo_id, 临时目录守卫)
fn setup() -> (GitDriverImpl, RepoId, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let driver = GitDriverImpl::new();
    block_on(driver.init_repo(tmp.path())).expect("init_repo");
    git_config(tmp.path(), "user.email", "test@ramag.dev");
    git_config(tmp.path(), "user.name", "Ramag Test");
    git_config(tmp.path(), "commit.gpgsign", "false");
    // open_repo 确保句柄注册（status 等按 RepoId 取句柄）
    let rc = block_on(driver.open_repo(tmp.path())).expect("open_repo");
    (driver, rc.id, tmp)
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

/// 写文件 + stage + commit 一条龙
fn commit_file(
    driver: &GitDriverImpl,
    id: &RepoId,
    dir: &Path,
    name: &str,
    content: &str,
    msg: &str,
) {
    write(dir, name, content);
    block_on(driver.stage(id, &[name.to_string()])).unwrap();
    block_on(driver.commit(id, msg, false, false)).unwrap();
}

/// 当前 HEAD 所在分支名（init 后默认分支名因 git 配置而异，动态取）
fn current_branch(driver: &GitDriverImpl, id: &RepoId) -> String {
    let branches = block_on(driver.list_branches(id, BranchKind::Local)).unwrap();
    branches
        .iter()
        .find(|b| b.is_head)
        .map(|b| b.name.clone())
        .expect("应有 HEAD 分支")
}

#[test]
fn init_stage_commit_log() {
    let (driver, id, tmp) = setup();
    write(tmp.path(), "a.txt", "line1\nline2\n");

    let st = block_on(driver.status(&id)).unwrap();
    assert_eq!(st.files.len(), 1, "应有 1 个 untracked");

    block_on(driver.stage(&id, &["a.txt".to_string()])).unwrap();
    let st = block_on(driver.status(&id)).unwrap();
    assert!(st.files[0].staged.is_some(), "stage 后应 staged");

    let cid = block_on(driver.commit(&id, "first commit", false, false)).unwrap();
    assert!(!cid.0.is_empty(), "commit 应返回非空 id");

    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "first commit");

    let st = block_on(driver.status(&id)).unwrap();
    assert!(st.files.is_empty(), "commit 后工作区应干净");
    assert!(st.head_branch.is_some(), "应有 HEAD 分支");
}

#[test]
fn unstage_and_discard() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");

    // 改 + stage，再 unstage
    write(tmp.path(), "a.txt", "base\nmore\n");
    block_on(driver.stage(&id, &["a.txt".to_string()])).unwrap();
    block_on(driver.unstage(&id, &["a.txt".to_string()])).unwrap();
    let st = block_on(driver.status(&id)).unwrap();
    let f = st.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert!(f.staged.is_none(), "unstage 后不应 staged");
    assert!(f.unstaged.is_some(), "改动仍在工作区");

    // discard 丢弃工作区改动
    block_on(driver.discard(&id, &["a.txt".to_string()])).unwrap();
    let st = block_on(driver.status(&id)).unwrap();
    assert!(st.files.is_empty(), "discard 后工作区应干净");
}

/// 行级部分暂存：只 stage 选中的新增行，其余改动留在工作区
#[test]
fn line_level_partial_stage() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "a\nb\nc\n", "init");

    // 加 X（b 前）和 Y（末尾）
    write(tmp.path(), "a.txt", "a\nX\nb\nc\nY\n");

    // 只 stage X 的行级 patch（Y 不 stage）—— build_patch_for_selection 的输出格式：
    // 选中的 add 保留 +，未选中的 add 省略，context 保留，真实 old_start 定位
    let patch =
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n a\n+X\n b\n c\n";
    block_on(driver.stage_patch(&id, patch)).expect("行级 stage_patch 应成功");

    let st = block_on(driver.status(&id)).unwrap();
    let f = st.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert!(f.staged.is_some(), "X 应进暂存区");
    assert!(f.unstaged.is_some(), "Y 应还在工作区未暂存");

    // index 内容应为 a\nX\nb\nc\n（含 X、不含 Y）
    let staged_diff = block_on(driver.diff_file(&id, "a.txt", DiffKind::IndexVsHead)).unwrap();
    let added: Vec<&str> = staged_diff
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .filter(|l| matches!(l.kind, ramag_domain::entities::DiffLineKind::Add))
        .map(|l| l.text.as_str())
        .collect();
    assert!(added.contains(&"X"), "暂存区应含 X，实际 {added:?}");
    assert!(!added.contains(&"Y"), "暂存区不应含 Y，实际 {added:?}");
}

/// 行级 unstage：从暂存区撤回选中行
#[test]
fn line_level_partial_unstage() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "a\nb\nc\n", "init");
    // 全量改 + stage
    write(tmp.path(), "a.txt", "a\nX\nb\nc\n");
    block_on(driver.stage(&id, &["a.txt".to_string()])).unwrap();
    // 行级 unstage X
    let patch =
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n a\n+X\n b\n c\n";
    block_on(driver.unstage_patch(&id, patch)).expect("行级 unstage_patch 应成功");
    let st = block_on(driver.status(&id)).unwrap();
    let f = st.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert!(f.unstaged.is_some(), "撤回后 X 回到工作区未暂存");
}

#[test]
fn branch_checkout_merge() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");
    let main = current_branch(&driver, &id);

    block_on(driver.create_branch(&id, "feature", None)).unwrap();
    block_on(driver.checkout(&id, "feature")).unwrap();
    commit_file(&driver, &id, tmp.path(), "b.txt", "feat\n", "feat commit");

    block_on(driver.checkout(&id, &main)).unwrap();
    block_on(driver.merge(&id, "feature", true, false, None)).unwrap();

    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert!(
        log.iter().any(|c| c.subject == "feat commit"),
        "merge 后历史应含 feature commit"
    );
}

#[test]
fn stash_save_apply() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");

    write(tmp.path(), "a.txt", "base\nmodified\n");
    block_on(driver.stash_save(&id, Some("wip"), false)).unwrap();
    let st = block_on(driver.status(&id)).unwrap();
    assert!(st.files.is_empty(), "stash 后工作区应干净");

    let stashes = block_on(driver.list_stashes(&id)).unwrap();
    assert_eq!(stashes.len(), 1, "应有 1 条 stash");

    block_on(driver.stash_apply(&id, 0, false)).unwrap();
    let st = block_on(driver.status(&id)).unwrap();
    assert!(!st.files.is_empty(), "apply 后改动应回来");
}

#[test]
fn tag_create_list() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");

    block_on(driver.create_tag(&id, "v1.0", None, Some("release"), false)).unwrap();
    let tags = block_on(driver.list_tags(&id)).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v1.0");
}

#[test]
fn reset_and_revert() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "v1\n", "c1");
    commit_file(&driver, &id, tmp.path(), "a.txt", "v2\n", "c2");

    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log.len(), 2);

    // revert 最新 commit（c2）
    let c2 = log[0].id.0.clone();
    block_on(driver.revert(&id, &c2)).unwrap();
    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log.len(), 3, "revert 应新增一个 commit");

    // reset --hard 回 c1
    let c1 = log.last().unwrap().id.0.clone();
    block_on(driver.reset(&id, &c1, ResetKind::Hard)).unwrap();
    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log.len(), 1, "reset 后只剩 c1");
}

#[test]
fn cherry_pick_commit() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");
    let main = current_branch(&driver, &id);

    block_on(driver.create_branch(&id, "feature", None)).unwrap();
    block_on(driver.checkout(&id, "feature")).unwrap();
    commit_file(&driver, &id, tmp.path(), "b.txt", "feat\n", "feat-commit");
    let feat = block_on(driver.log(&id, LogOptions::default())).unwrap()[0]
        .id
        .0
        .clone();

    block_on(driver.checkout(&id, &main)).unwrap();
    block_on(driver.cherry_pick(&id, &feat)).unwrap();
    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert!(log.iter().any(|c| c.subject == "feat-commit"));
}

#[test]
fn diff_and_blame() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "l1\nl2\n", "init");
    write(tmp.path(), "a.txt", "l1\nl2\nl3\n");

    let diff = block_on(driver.diff_file(&id, "a.txt", DiffKind::WorkingTreeVsIndex)).unwrap();
    assert!(!diff.hunks.is_empty(), "diff 应有 hunk");

    block_on(driver.stage(&id, &["a.txt".to_string()])).unwrap();
    block_on(driver.commit(&id, "c2", false, false)).unwrap();
    let blame = block_on(driver.blame(&id, "a.txt")).unwrap();
    assert_eq!(blame.len(), 3, "blame 行数应等于文件行数");
}

#[test]
fn merge_conflict_detected() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");
    let main = current_branch(&driver, &id);

    block_on(driver.create_branch(&id, "feature", None)).unwrap();
    block_on(driver.checkout(&id, "feature")).unwrap();
    commit_file(
        &driver,
        &id,
        tmp.path(),
        "a.txt",
        "feature-change\n",
        "feat",
    );

    block_on(driver.checkout(&id, &main)).unwrap();
    commit_file(&driver, &id, tmp.path(), "a.txt", "main-change\n", "main");

    // 冲突 merge：应返回 Err 或进入冲突状态（status.operation = Merge）
    let _ = block_on(driver.merge(&id, "feature", false, false, None));
    let st = block_on(driver.status(&id)).unwrap();
    let has_conflict = st.files.iter().any(|f| {
        matches!(
            f.staged,
            Some(ramag_domain::entities::FileChangeKind::Conflicted)
        )
    });
    assert!(
        has_conflict || st.operation.is_some(),
        "冲突 merge 后应检测到冲突文件或进行中操作"
    );
}

#[test]
fn rebase_onto_branch() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "c1");
    let main = current_branch(&driver, &id);

    block_on(driver.create_branch(&id, "feature", None)).unwrap();
    block_on(driver.checkout(&id, "feature")).unwrap();
    commit_file(&driver, &id, tmp.path(), "f.txt", "feat\n", "feat");

    block_on(driver.checkout(&id, &main)).unwrap();
    commit_file(&driver, &id, tmp.path(), "m.txt", "main\n", "main-commit");

    block_on(driver.checkout(&id, "feature")).unwrap();
    block_on(driver.rebase(&id, &main)).expect("rebase 应成功");
    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert!(
        log.iter().any(|c| c.subject == "main-commit"),
        "rebase 后 feature 应含 main 的 commit"
    );
    assert!(log.iter().any(|c| c.subject == "feat"));
}

/// interactive rebase：drop 中间 commit。验证 execute（上次改过 stderr 判定）真机可用
#[test]
fn interactive_rebase_drop() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "1\n", "c1");
    commit_file(&driver, &id, tmp.path(), "b.txt", "2\n", "c2");
    commit_file(&driver, &id, tmp.path(), "c.txt", "3\n", "c3");

    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    let c1 = log.last().unwrap().id.0.clone();

    let mut plan = block_on(driver.interactive_rebase_plan(&id, &c1)).unwrap();
    assert_eq!(plan.len(), 2, "c1..HEAD 应有 c2,c3");
    // plan 最老在前：plan[0]=c2，标记 Drop
    plan[0].action = ramag_domain::entities::RebaseAction::Drop;
    block_on(driver.interactive_rebase_execute(&id, &c1, &plan))
        .expect("interactive rebase execute");

    let log2 = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log2.len(), 2, "drop c2 后应剩 c1,c3");
    assert!(!log2.iter().any(|c| c.subject == "c2"), "c2 应被 drop");
    assert!(log2.iter().any(|c| c.subject == "c3"), "c3 应保留");
}

#[test]
fn remote_add_list() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");
    block_on(driver.add_remote(&id, "origin", "https://example.com/r.git")).unwrap();
    let remotes = block_on(driver.list_remotes(&id)).unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "origin");
}

#[test]
fn reflog_records_commits() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "v1\n", "c1");
    commit_file(&driver, &id, tmp.path(), "a.txt", "v2\n", "c2");
    let reflog = block_on(driver.list_reflog(&id, None, Some(50))).unwrap();
    assert!(
        reflog.len() >= 2,
        "reflog 应记录 commit 操作，实际 {}",
        reflog.len()
    );
}

#[test]
fn list_files_and_commit_files() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "x\n", "c1");
    write(tmp.path(), "b.txt", "y\n"); // untracked

    let files = block_on(driver.list_files(&id)).unwrap();
    assert!(files.contains(&"a.txt".to_string()), "应含 tracked a.txt");
    assert!(
        files.contains(&"b.txt".to_string()),
        "list_files 应含 untracked b.txt"
    );

    let cid = block_on(driver.log(&id, LogOptions::default())).unwrap()[0]
        .id
        .0
        .clone();
    let cf = block_on(driver.list_commit_files(&id, &cid)).unwrap();
    assert!(
        cf.iter().any(|f| f.path == "a.txt"),
        "commit 文件应含 a.txt"
    );
}

/// diff_file 内容 + 行号映射精确正确（diff 渲染的输入，保证 UI 不会行错位）
#[test]
fn diff_content_and_line_numbers_precise() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "a\nb\nc\n", "init");
    write(tmp.path(), "a.txt", "a\nB\nc\n"); // b → B

    let diff = block_on(driver.diff_file(&id, "a.txt", DiffKind::WorkingTreeVsIndex)).unwrap();
    let lines: Vec<&ramag_domain::entities::DiffLine> =
        diff.hunks.iter().flat_map(|h| &h.lines).collect();

    let del = lines.iter().find(|l| l.text == "b").expect("应有删除行 b");
    assert!(matches!(del.kind, DiffLineKind::Delete), "b 应为 Delete");
    assert!(
        del.old_lineno == Some(2) && del.new_lineno.is_none(),
        "删除行：有 old_lineno(2) 无 new_lineno，实际 old={:?} new={:?}",
        del.old_lineno,
        del.new_lineno
    );

    let add = lines.iter().find(|l| l.text == "B").expect("应有新增行 B");
    assert!(matches!(add.kind, DiffLineKind::Add), "B 应为 Add");
    assert!(
        add.new_lineno == Some(2) && add.old_lineno.is_none(),
        "新增行：有 new_lineno(2) 无 old_lineno，实际 old={:?} new={:?}",
        add.old_lineno,
        add.new_lineno
    );

    // context 行 a/c 两侧行号都在
    let ctx_a = lines
        .iter()
        .find(|l| l.text == "a")
        .expect("应有 context a");
    assert!(
        ctx_a.old_lineno == Some(1) && ctx_a.new_lineno == Some(1),
        "context a 两侧行号应为 1"
    );
}

fn adds_of(diff: &ramag_domain::entities::FileDiff) -> Vec<String> {
    diff.hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l.kind, DiffLineKind::Add))
        .map(|l| l.text.clone())
        .collect()
}

fn dels_of(diff: &ramag_domain::entities::FileDiff) -> Vec<String> {
    diff.hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l.kind, DiffLineKind::Delete))
        .map(|l| l.text.clone())
        .collect()
}

/// 根 commit 的文件 diff：之前 `git diff <c>^ <c>` 对无父 commit 报错，点第一个 commit 看 diff 失败
#[test]
fn diff_root_commit_file() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "l1\nl2\nl3\n", "root");
    let cid = block_on(driver.log(&id, LogOptions::default())).unwrap()[0]
        .id
        .0
        .clone();

    let diff = block_on(driver.diff_file(&id, "a.txt", DiffKind::CommitVsParent(CommitId(cid))))
        .expect("根 commit 的文件 diff 应成功（不再因 <c>^ 不存在而报错）");
    assert_eq!(
        adds_of(&diff),
        vec!["l1", "l2", "l3"],
        "根 commit diff 应把所有行显示为新增"
    );
}

/// 普通（有父）commit 的文件 diff
#[test]
fn diff_normal_commit_file() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "v1\n", "c1");
    commit_file(&driver, &id, tmp.path(), "a.txt", "v2\n", "c2");
    let cid = block_on(driver.log(&id, LogOptions::default())).unwrap()[0]
        .id
        .0
        .clone();

    let diff =
        block_on(driver.diff_file(&id, "a.txt", DiffKind::CommitVsParent(CommitId(cid)))).unwrap();
    assert_eq!(dels_of(&diff), vec!["v1"], "c2 应删 v1");
    assert_eq!(adds_of(&diff), vec!["v2"], "c2 应增 v2");
}

/// 大文件首尾两处改动 → 应分成 2 个 hunk，行号连续正确（验证多 hunk 解析）
#[test]
fn diff_large_file_multi_hunk() {
    let (driver, id, tmp) = setup();
    let content: String = (1..=50).map(|i| format!("line{i}\n")).collect();
    commit_file(&driver, &id, tmp.path(), "big.txt", &content, "init");

    let mut lines: Vec<String> = (1..=50).map(|i| format!("line{i}")).collect();
    lines[1] = "LINE2".into(); // 改第 2 行
    lines[48] = "LINE49".into(); // 改第 49 行
    write(tmp.path(), "big.txt", &(lines.join("\n") + "\n"));

    let diff = block_on(driver.diff_file(&id, "big.txt", DiffKind::WorkingTreeVsIndex)).unwrap();
    assert_eq!(diff.hunks.len(), 2, "相距很远的两处改动应分成 2 个 hunk");
    assert!(
        diff.hunks[1].old_start > 40,
        "第二个 hunk 行号应接近 49，实际 {}",
        diff.hunks[1].old_start
    );
    assert!(adds_of(&diff).contains(&"LINE49".to_string()));
}

/// 纯新增文件（split 渲染会退化 unified）
#[test]
fn diff_pure_add_file() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "base\n", "init");
    write(tmp.path(), "new.txt", "n1\nn2\n");
    block_on(driver.stage(&id, &["new.txt".to_string()])).unwrap();

    let diff = block_on(driver.diff_file(&id, "new.txt", DiffKind::IndexVsHead)).unwrap();
    assert_eq!(diff.change_kind, FileChangeKind::Added);
    assert_eq!(adds_of(&diff), vec!["n1", "n2"]);
}

/// 无换行结尾文件：git diff 输出 `\ No newline at end of file`，解析须忽略该标记不当成内容行
#[test]
fn diff_no_newline_at_eof() {
    let (driver, id, tmp) = setup();
    commit_file(&driver, &id, tmp.path(), "a.txt", "l1\nl2\n", "init");
    std::fs::write(tmp.path().join("a.txt"), "l1\nl2\nl3").unwrap(); // 末尾无换行

    let diff = block_on(driver.diff_file(&id, "a.txt", DiffKind::WorkingTreeVsIndex)).unwrap();
    assert!(
        adds_of(&diff).contains(&"l3".to_string()),
        "应识别新增行 l3"
    );
    let all: Vec<String> = diff
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| l.text.clone())
        .collect();
    assert!(
        !all.iter().any(|t| t.contains("No newline")),
        "`\\ No newline` 标记不应被当成内容行，实际 {all:?}"
    );
}

/// amend：空 message 保留原 commit message；非空 message 改写。两种都不应新增 commit 数
#[test]
fn amend_keeps_or_rewrites_message() {
    let (driver, id, tmp) = setup();
    commit_file(
        &driver,
        &id,
        tmp.path(),
        "a.txt",
        "v1\n",
        "original message",
    );

    // 空 message amend：补一个文件进上一个 commit，message 不变
    write(tmp.path(), "b.txt", "extra\n");
    block_on(driver.stage(&id, &["b.txt".to_string()])).unwrap();
    block_on(driver.commit(&id, "", true, false)).expect("空 message amend 应成功");
    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log.len(), 1, "amend 不应新增 commit");
    assert_eq!(log[0].subject, "original message", "空 message 应保留原文");

    // 非空 message amend：改写 message
    block_on(driver.commit(&id, "rewritten", true, false)).expect("amend 改 message 应成功");
    let log = block_on(driver.log(&id, LogOptions::default())).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "rewritten");
}
