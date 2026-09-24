use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, anyhow};
use jj_lib::backend::CommitId;
use jj_lib::default_backend_factories::{
    default_backend_factories, default_working_copy_factories,
};
use jj_lib::fileset::FilesetAliasesMap;
use jj_lib::git::{
    GitFetch, GitFetchRefExpression, GitImportOptions, GitImportStats, GitProgress, GitPushOptions,
    GitPushRefTargets, GitPushStats, GitSettings, GitSidebandLineTerminator, GitSubprocessCallback,
    expand_fetch_refspecs, load_default_fetch_bookmarks, push_refs,
};
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::{RemoteName, RemoteNameBuf};
use jj_lib::refs::{RefPushAction, classify_ref_push_action};
use jj_lib::repo::Repo as _;
use jj_lib::revset::{
    RevsetAliasesMap, RevsetDiagnostics, RevsetExtensions, RevsetParseContext,
    RevsetWorkspaceContext, SymbolResolver, parse, parse_string_expression,
};
use jj_lib::settings::RemoteSettingsMap;
use jj_lib::str_util::{StringExpression, StringMatcher};
use jj_lib::time_util::DatePatternContext;
use jj_lib::ui_path::RepoPathUiConverter;
use jj_lib::workspace::Workspace;
use pollster::FutureExt as _;

pub mod user_config;
pub mod user_settings;

pub const DEFAULT_REVSET: &str = "present(@) | ancestors(immutable_heads().., 2) | trunk()";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitHistoryItem {
    pub id: String,
    pub change_id: String,
    pub description: String,
    pub author: String,
    pub timestamp: String,
    pub parents: Vec<String>,
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub name: String,
    pub kind: BookmarkKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarkKind {
    Local,
    Remote(String),
}

#[derive(Debug, Clone)]
pub struct CommitHistory {
    pub commits: Vec<CommitHistoryItem>,
    pub working_copy_commit_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitBackendInfo {
    pub branch: Option<String>,
}

fn load_workspace(path: &Path) -> anyhow::Result<Workspace> {
    let settings = user_settings::create_user_settings(Some(path))?;
    let store_factories = default_backend_factories();
    let working_copy_factories = default_working_copy_factories();

    Workspace::load(&settings, path, &store_factories, &working_copy_factories).map_err(Into::into)
}

pub fn is_repo<P: AsRef<Path>>(path: P) -> bool {
    load_workspace(path.as_ref()).is_ok()
}

pub fn git_backend_info<P: AsRef<Path>>(path: P) -> Option<GitBackendInfo> {
    let workspace = load_workspace(path.as_ref()).ok()?;
    let repo = workspace.repo_loader().load_at_head().block_on().ok()?;
    let git_repo = jj_lib::git::get_git_repo(repo.store()).ok()?;
    let branch = git_repo
        .head_name()
        .ok()
        .flatten()
        .map(|name| name.shorten().to_string());

    Some(GitBackendInfo { branch })
}

pub fn init_git_backend<P: AsRef<Path>>(path: P) -> anyhow::Result<()> {
    let path = path.as_ref();
    if path.join(".jj").exists() {
        anyhow::bail!("该项目已经是 jj 仓库，但不是 Git backend；当前不能原地转换")
    }

    let settings = user_settings::create_user_settings(Some(path))?;
    if !path.join(".git").exists() {
        let status = Command::new("git")
            .arg("init")
            .arg("--")
            .arg(path)
            .status()
            .context("failed to run git init")?;
        if !status.success() {
            anyhow::bail!("git init exited with status {status}");
        }
    }

    Workspace::init_external_git(&settings, path, &path.join(".git"))
        .block_on()
        .context("failed to initialize jj workspace with the Git repository")?;

    Ok(())
}

pub fn load_history<P: AsRef<Path>>(path: P, revset_str: &str) -> anyhow::Result<CommitHistory> {
    let path = path.as_ref();
    let workspace = load_workspace(path)?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .block_on()
        .context("failed to load repository")?;
    let working_copy_commit_id = repo
        .view()
        .get_wc_commit_id(workspace.workspace_name())
        .map(ToString::to_string);

    let mut aliases = RevsetAliasesMap::new();
    let alias_names = repo
        .settings()
        .config()
        .table_keys("revset-aliases")
        .collect::<Vec<_>>();
    for name in alias_names {
        let definition = repo.settings().get_string(["revset-aliases", name])?;
        let definition = if definition.trim() == "builtin_immutable_heads()" {
            "trunk() | tags() | untracked_remote_bookmarks()".to_owned()
        } else {
            definition
        };
        aliases
            .insert(name, definition, None)
            .map_err(|error| anyhow!("invalid revset alias {name}: {error}"))?;
    }
    if aliases.get_function("immutable_heads", 0).is_none() {
        aliases.insert(
            "immutable_heads()",
            "trunk() | tags() | untracked_remote_bookmarks()",
            None,
        )?;
    }
    if aliases.get_function("trunk", 0).is_none() {
        aliases.insert(
            "trunk()",
            "coalesce(remote_bookmarks(exact:\"main\", remote=\"origin\"), remote_bookmarks(exact:\"master\", remote=\"origin\"), bookmarks(exact:\"main\"), bookmarks(exact:\"master\"), root())",
            None,
        )?;
    }

    let path_converter = RepoPathUiConverter::Fs {
        cwd: path.to_owned(),
        base: path.to_owned(),
    };
    let workspace_context = RevsetWorkspaceContext {
        path_converter: &path_converter,
        workspace_name: workspace.workspace_name(),
    };
    let fileset_aliases = FilesetAliasesMap::new();
    let extensions = RevsetExtensions::default();
    let context = RevsetParseContext {
        aliases_map: &aliases,
        local_variables: HashMap::new(),
        user_email: repo.settings().user_email(),
        date_pattern_context: DatePatternContext::Local(chrono::Local::now()),
        default_ignored_remote: None,
        fileset_aliases_map: &fileset_aliases,
        extensions: &extensions,
        workspace: Some(workspace_context),
    };
    let expression = parse(&mut RevsetDiagnostics::new(), revset_str, &context)
        .context("failed to parse revset")?;
    let symbol_resolver = SymbolResolver::new(repo.as_ref(), extensions.symbol_resolvers());
    let revset = expression
        .resolve_user_expression(repo.as_ref(), &symbol_resolver)
        .context("failed to resolve revset")?
        .evaluate(repo.as_ref())
        .context("failed to evaluate repository history")?;
    let graph_nodes = futures::executor::block_on(async {
        let mut stream = revset.stream_graph();
        let mut nodes = Vec::new();
        while let Some(node) = futures::StreamExt::next(&mut stream).await {
            nodes.push(node?);
        }
        Ok::<_, jj_lib::revset::RevsetEvaluationError>(nodes)
    })
    .context("failed to read repository history")?;

    // Keep each fork together before rendering it. This is the same grouping
    // strategy used by jj's graph output and prevents independent lanes from
    // weaving across each other as they converge.
    let graph_nodes = futures::executor::block_on(async {
        let stream = jj_lib::graph::TopoGroupedGraph::new(
            futures::stream::iter(graph_nodes.into_iter().map(Ok::<_, anyhow::Error>)),
            |id: &CommitId| id,
        )
        .stream();
        futures::pin_mut!(stream);
        let mut nodes = Vec::new();
        while let Some(node) = futures::StreamExt::next(&mut stream).await {
            nodes.push(node?);
        }
        Ok::<_, anyhow::Error>(nodes)
    })?;

    let mut bookmarks_by_commit: HashMap<String, Vec<Bookmark>> = HashMap::new();
    for (name, target) in repo.view().local_bookmarks() {
        for commit_id in target.added_ids() {
            bookmarks_by_commit
                .entry(commit_id.to_string())
                .or_default()
                .push(Bookmark {
                    name: name.as_str().to_owned(),
                    kind: BookmarkKind::Local,
                });
        }
    }
    for (symbol, remote_ref) in repo.view().all_remote_bookmarks() {
        for commit_id in remote_ref.target.added_ids() {
            bookmarks_by_commit
                .entry(commit_id.to_string())
                .or_default()
                .push(Bookmark {
                    name: symbol.name.as_str().to_owned(),
                    kind: BookmarkKind::Remote(symbol.remote.as_str().to_owned()),
                });
        }
    }

    let commits = graph_nodes
        .into_iter()
        .map(|(commit_id, _edges)| {
            let commit = repo.store().get_commit(&commit_id)?;
            let timestamp = commit
                .author()
                .timestamp
                .to_datetime()
                .map(|date| date.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|_| "unknown time".to_owned());

            Ok(CommitHistoryItem {
                id: commit.id().to_string(),
                change_id: commit.change_id().to_string(),
                description: commit
                    .description()
                    .lines()
                    .next()
                    .unwrap_or("(no description)")
                    .to_owned(),
                author: format!("{} <{}>", commit.author().name, commit.author().email),
                timestamp,
                parents: commit
                    .parent_ids()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                bookmarks: bookmarks_by_commit
                    .remove(&commit.id().to_string())
                    .unwrap_or_default(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(CommitHistory {
        commits,
        working_copy_commit_id,
    })
}

/// 接住 git 子进程说过的话。
///
/// 界面里没有实时显示进度的地方，所以只在最后几行有意义的话上留个记录：
/// 失败时拼进错误里，「认证失败」「远程已经变过」这类原因就看得见了。
#[derive(Default)]
struct GitOutput {
    lines: Vec<String>,
}

/// 最多留几行 git 输出，够说明问题就行。
const MAX_GIT_OUTPUT_LINES: usize = 6;

impl GitOutput {
    fn push_line(&mut self, prefix: &str, message: &[u8]) {
        let text = String::from_utf8_lossy(message);
        let text = text.trim();
        if text.is_empty() {
            return;
        }

        self.lines.push(format!("{prefix}{text}"));
        if self.lines.len() > MAX_GIT_OUTPUT_LINES {
            self.lines.remove(0);
        }
    }

    /// 拼在错误后面用；git 什么都没说就返回空串。
    fn detail(&self) -> String {
        if self.lines.is_empty() {
            String::new()
        } else {
            format!("\n{}", self.lines.join("\n"))
        }
    }
}

impl GitSubprocessCallback for GitOutput {
    fn needs_progress(&self) -> bool {
        false
    }

    fn progress(&mut self, _progress: &GitProgress) -> io::Result<()> {
        Ok(())
    }

    fn local_sideband(
        &mut self,
        message: &[u8],
        _terminator: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        self.push_line("git: ", message);
        Ok(())
    }

    fn remote_sideband(
        &mut self,
        message: &[u8],
        _terminator: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        self.push_line("remote: ", message);
        Ok(())
    }
}

/// 把 `remotes.<name>.fetch-bookmarks` 这类配置里的名称模式解析成表达式。
///
/// 语法和 jj 命令行一致（`*` 是 glob），所以直接借 jj-lib 的解析器。
fn parse_name_pattern(text: &str) -> anyhow::Result<StringExpression> {
    let mut diagnostics = RevsetDiagnostics::new();
    parse_string_expression(&mut diagnostics, text)
        .map_err(|error| anyhow!("无效的名称模式 {text}：{error}"))
}

/// `remotes.<name>.auto-track-bookmarks`：拉取时哪些 bookmark 要自动 track。
///
/// 没配就是都不自动 track，和 jj 命令行一样；不配的话拉下来的新 bookmark
/// 是「未跟踪」状态，要显式 track 过才推得回远程。
fn auto_track_bookmarks(
    remote_settings: &RemoteSettingsMap,
) -> anyhow::Result<HashMap<RemoteNameBuf, StringMatcher>> {
    let mut matchers = HashMap::new();
    for (name, settings) in remote_settings {
        let Some(text) = settings.auto_track_bookmarks.as_deref() else {
            continue;
        };

        matchers.insert(name.clone(), parse_name_pattern(text)?.to_matcher());
    }

    Ok(matchers)
}

/// 拉取结果的摘要，带上是哪个远程，界面直接拿它当通知正文。
fn summarize_import(stats: &GitImportStats, ignored_refspecs: usize, remote: &str) -> String {
    let mut parts = Vec::new();
    if !stats.changed_remote_bookmarks.is_empty() {
        parts.push(format!(
            "更新了 {} 个远程 bookmark",
            stats.changed_remote_bookmarks.len()
        ));
    }
    if !stats.changed_remote_tags.is_empty() {
        parts.push(format!(
            "更新了 {} 个远程 tag",
            stats.changed_remote_tags.len()
        ));
    }
    if !stats.abandoned_commits.is_empty() {
        parts.push(format!(
            "{} 个提交变成不可达",
            stats.abandoned_commits.len()
        ));
    }
    if !stats.failed_ref_names.is_empty() {
        parts.push(format!("{} 个 ref 没能导入", stats.failed_ref_names.len()));
    }
    if ignored_refspecs > 0 {
        parts.push(format!("{ignored_refspecs} 条 refspec 被忽略"));
    }

    if parts.is_empty() {
        parts.push("远程没有新东西".to_owned());
    }
    format!("{remote}：{}", parts.join("，"))
}

/// 推送结果的摘要，带上是哪个远程。
///
/// 推空、被拒、跳过都要分开说：jj 允许一部分 ref 成功一部分被拒，只说「已推送」
/// 会把被拒和跳过的那些盖掉，而它们恰恰是用户要知道的。
fn summarize_push(stats: &GitPushStats, skipped: &[String], remote: &str) -> String {
    let mut parts = Vec::new();
    if !stats.pushed.is_empty() {
        parts.push(format!("已推送 {} 个 bookmark", stats.pushed.len()));
    }
    if !stats.rejected.is_empty() {
        parts.push(format!(
            "{} 个因远程已变动被拒（先拉取再推）",
            stats.rejected.len()
        ));
    }
    if !stats.remote_rejected.is_empty() {
        parts.push(format!("{} 个被远程拒绝", stats.remote_rejected.len()));
    }
    if !stats.unexported_bookmarks.is_empty() {
        parts.push(format!(
            "{} 个没能写回本地 git 仓库",
            stats.unexported_bookmarks.len()
        ));
    }
    if !skipped.is_empty() {
        parts.push(format!("跳过 {}", skipped.join("、")));
    }

    // 一件事都没发生（也没被拒、没跳过）才叫「没东西可推」。
    if parts.is_empty() {
        parts.push("没有 bookmark 需要推送".to_owned());
    }
    format!("{remote}：{}", parts.join("，"))
}

/// 读仓库里配置的所有 git remote 名，按名字排序。
///
/// 只是读配置、不碰网络，所以还是走 jj-lib，和读提交历史保持一致。
pub fn load_remotes<P: AsRef<Path>>(path: P) -> anyhow::Result<Vec<String>> {
    let workspace = load_workspace(path.as_ref())?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .block_on()
        .context("failed to load repository")?;
    // 不是 Git backend 的仓库本来就没有远程仓库，这种「读不到」按空列表处理。
    let mut remotes = jj_lib::git::get_all_remote_names(repo.store())
        .map(|names| {
            names
                .into_iter()
                .map(|name| name.as_str().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    remotes.sort();

    Ok(remotes)
}

/// 拉取远程：把远程的 bookmark 和 tag 取回本地，更新仓库视图。
///
/// 走 jj-lib 而不是 `jj` 命令行：启动器不需要用户装 jj，取哪些 ref、自动跟踪哪些
/// bookmark 都照 jj 的默认规则来（`remotes.<name>.fetch-bookmarks` 这类配置一样生效）。
/// 返回值是给界面用的一句话摘要，直接当通知正文。
pub fn fetch_remote<P: AsRef<Path>>(path: P, remote: &str) -> anyhow::Result<String> {
    let path = path.as_ref();
    let settings = user_settings::create_user_settings(Some(path))?;
    let git_settings = GitSettings::from_settings(&settings)?;
    let remote_settings = settings.remote_settings()?;
    let remote_name = RemoteName::new(remote);

    let mut workspace = load_workspace(path)?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .block_on()
        .context("failed to load repository")?;
    // 先记下当前工作副本：导入远程 ref 有可能把它换掉，换掉就要跟着切过去。
    let old_wc_commit_id = repo
        .view()
        .get_wc_commit_id(workspace.workspace_name())
        .cloned();

    // bookmark 默认按 remote 自己的 refspec 取，tag 取全部；两者都能被
    // `remotes.<name>.fetch-bookmarks` / `fetch-tags` 覆盖，和 jj 命令行一致。
    let git_repo =
        jj_lib::git::get_git_repo(repo.store()).context("repository is not Git-backed")?;
    let (ignored_refspecs, default_bookmark_expr) =
        load_default_fetch_bookmarks(remote_name, &git_repo)?;
    let remote_config = remote_settings.get(remote_name);
    let bookmark_expr = match remote_config.and_then(|config| config.fetch_bookmarks.as_deref()) {
        Some(text) => parse_name_pattern(text)?,
        None => default_bookmark_expr,
    };
    let tag_expr = match remote_config.and_then(|config| config.fetch_tags.as_deref()) {
        Some(text) => parse_name_pattern(text)?,
        None => StringExpression::all(),
    };
    let expanded = expand_fetch_refspecs(
        remote_name,
        GitFetchRefExpression {
            bookmark: bookmark_expr,
            tag: tag_expr,
        },
    )?;
    let import_options = GitImportOptions {
        abandon_unreachable_commits: git_settings.abandon_unreachable_commits,
        record_synthetic_predecessors: git_settings.record_synthetic_predecessors,
        remote_auto_track_bookmarks: auto_track_bookmarks(&remote_settings)?,
    };

    let mut git_output = GitOutput::default();
    let mut transaction = repo.start_transaction();
    let stats = {
        let mut fetch = GitFetch::new(
            transaction.repo_mut(),
            git_settings.to_subprocess_options(),
            &import_options,
        )?;
        fetch
            .fetch(remote_name, expanded, &mut git_output, None)
            .map_err(|error| anyhow!("{error}{}", git_output.detail()))?;
        fetch
            .import_refs()
            .block_on()
            .map_err(|error| anyhow!("{error}{}", git_output.detail()))?
    };
    let repo = transaction
        .commit(format!("fetch from git remote {remote}"))
        .block_on()
        .context("failed to save fetch result")?;

    // 不可达提交被放弃时，被放弃的可能正是工作副本那个提交，得把工作副本
    // 切到新的提交上，不然磁盘上的文件和仓库状态就对不上了。
    let new_wc_commit_id = repo
        .view()
        .get_wc_commit_id(workspace.workspace_name())
        .cloned();
    if new_wc_commit_id != old_wc_commit_id
        && let Some(commit_id) = new_wc_commit_id
    {
        let commit = repo
            .store()
            .get_commit(&commit_id)
            .context("failed to read the new working-copy commit")?;
        workspace
            .check_out(repo.op_id().clone(), None, &commit)
            .block_on()
            .context("failed to update working copy")?;
    }

    Ok(summarize_import(&stats, ignored_refspecs.0.len(), remote))
}

/// 推送：把本地所有 bookmark 推到远程。
///
/// 远程上还不存在的 bookmark 也会新建，和 `jj git push --all` 的范围一致。每个 ref
/// 的「远程原来在哪」取自本地记录的远程状态，jj-lib 会拿它跟远程对账，对不上就拒绝，
/// 所以不会把别人后来推上去的提交盖掉。返回值同 [`fetch_remote`]。
pub fn push_remote<P: AsRef<Path>>(path: P, remote: &str) -> anyhow::Result<String> {
    let path = path.as_ref();
    let settings = user_settings::create_user_settings(Some(path))?;
    let git_settings = GitSettings::from_settings(&settings)?;
    let remote_name = RemoteName::new(remote);

    let workspace = load_workspace(path)?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .block_on()
        .context("failed to load repository")?;
    let mut transaction = repo.start_transaction();

    // 挑出要推的 ref。分类直接用 jj-lib 的，和 jj 命令行是同一套判断：
    // 推不动的（本地有冲突、远程那份有冲突或者没被 track）不硬推，把原因带回界面。
    let mut targets = GitPushRefTargets::default();
    let mut skipped = Vec::new();
    for (name, local_remote) in transaction
        .repo()
        .view()
        .local_remote_bookmarks(remote_name)
    {
        let bookmark = name.as_str().to_owned();
        match classify_ref_push_action(local_remote) {
            RefPushAction::AlreadyMatches => {}
            // after 为空说明这个 bookmark 本地已经删了；这里不带删除，跳过。
            RefPushAction::Update(update) if update.after.is_none() => {
                skipped.push(format!("{bookmark}（本地已删除）"));
            }
            RefPushAction::Update(update) => targets.bookmarks.push((name.to_owned(), update)),
            RefPushAction::LocalConflicted => skipped.push(format!("{bookmark}（本地有冲突）")),
            RefPushAction::RemoteConflicted => {
                skipped.push(format!("{bookmark}（远程那份有冲突，先拉取）"));
            }
            RefPushAction::RemoteUntracked => {
                skipped.push(format!("{bookmark}（远程已有但本地没跟踪）"));
            }
        }
    }

    if targets.bookmarks.is_empty() {
        return Ok(summarize_push(&GitPushStats::default(), &skipped, remote));
    }

    let mut git_output = GitOutput::default();
    let stats = push_refs(
        transaction.repo_mut(),
        git_settings.to_subprocess_options(),
        remote_name,
        &targets,
        &mut git_output,
        &GitPushOptions::default(),
    )
    .map_err(|error| anyhow!("{error}{}", git_output.detail()))?;

    // 有推成功的就先把结果落盘（jj 命令行也是这个顺序）：一部分被拒时，
    // 成功的那部分不能白推，不然下次还会被当成没推过。
    if stats.all_ok() || stats.some_exported() {
        transaction
            .commit(format!("push all bookmarks to git remote {remote}"))
            .block_on()
            .context("failed to save push result")?;
    }

    let summary = summarize_push(&stats, &skipped, remote);
    if stats.all_ok() {
        Ok(summary)
    } else {
        Err(anyhow!(summary))
    }
}

pub fn checkout<P: AsRef<Path>>(path: P, commit_id: &str) -> anyhow::Result<()> {
    let mut workspace = load_workspace(path.as_ref())?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .block_on()
        .context("failed to load repository")?;
    let commit_id = CommitId::try_from_hex(commit_id.as_bytes())
        .ok_or_else(|| anyhow!("invalid commit id: {commit_id}"))?;
    let commit = repo
        .store()
        .get_commit(&commit_id)
        .context("commit does not exist")?;

    let mut transaction = repo.start_transaction();
    transaction
        .repo_mut()
        .edit(workspace.workspace_name().to_owned(), &commit)
        .block_on()
        .context("failed to select commit")?;
    let repo = transaction
        .commit(format!("checkout commit {commit_id}"))
        .block_on()
        .context("failed to save checkout")?;
    let commit = repo.store().get_commit(&commit_id)?;

    workspace
        .check_out(repo.op_id().clone(), None, &commit)
        .block_on()
        .context("failed to update working copy")?;
    Ok(())
}

pub fn move_local_bookmark<P: AsRef<Path>>(
    path: P,
    bookmark_name: &str,
    commit_id: &str,
) -> anyhow::Result<()> {
    let workspace = load_workspace(path.as_ref())?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .block_on()
        .context("failed to load repository")?;
    let commit_id = CommitId::try_from_hex(commit_id.as_bytes())
        .ok_or_else(|| anyhow!("invalid commit id: {commit_id}"))?;
    let commit = repo
        .store()
        .get_commit(&commit_id)
        .context("commit does not exist")?;

    let mut transaction = repo.start_transaction();
    transaction.repo_mut().set_local_bookmark_target(
        jj_lib::ref_name::RefName::new(bookmark_name),
        RefTarget::normal(commit.id().clone()),
    );
    transaction
        .commit(format!("move bookmark {bookmark_name} to {commit_id}"))
        .block_on()
        .context("failed to save bookmark")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jj_lib::git::GitImportRefUpdate;
    use jj_lib::op_store::RemoteRef;
    use jj_lib::ref_name::{GitRefNameBuf, RefNameBuf};
    use jj_lib::settings::RemoteSettings;

    #[test]
    fn test_is_repo() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(is_repo(workspace));
    }

    #[test]
    fn test_load_history() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let history =
            load_history(workspace, DEFAULT_REVSET).expect("the workspace history should load");
        assert!(!history.commits.is_empty());
    }

    #[test]
    fn test_load_remotes() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let remotes = load_remotes(workspace).expect("the workspace remotes should load");

        // 本仓库就是从 GitHub 上的 origin 克隆出来的，列表里应该有它。
        assert!(remotes.iter().any(|remote| remote == "origin"));
    }

    #[test]
    fn summarize_import_reports_what_changed_and_says_so_when_nothing_did() {
        // 什么都没变的时候要说清楚是「没有新东西」，不能让人以为拉到了什么。
        assert_eq!(
            summarize_import(&GitImportStats::default(), 0, "origin"),
            "origin：远程没有新东西"
        );

        let symbol = RefNameBuf::from("main")
            .to_remote_symbol(&RemoteNameBuf::from("origin"))
            .to_owned();
        let stats = GitImportStats {
            changed_remote_bookmarks: vec![GitImportRefUpdate::new(
                symbol,
                RemoteRef::absent(),
                RefTarget::normal(
                    CommitId::try_from_hex(b"0123456789abcdef0123456789abcdef01234567").unwrap(),
                ),
            )],
            ..Default::default()
        };
        assert_eq!(
            summarize_import(&stats, 0, "origin"),
            "origin：更新了 1 个远程 bookmark"
        );
        assert_eq!(
            summarize_import(&stats, 1, "origin"),
            "origin：更新了 1 个远程 bookmark，1 条 refspec 被忽略"
        );
    }

    #[test]
    fn summarize_push_mentions_rejections_and_skipped_refs() {
        assert_eq!(
            summarize_push(&GitPushStats::default(), &[], "origin"),
            "origin：没有 bookmark 需要推送"
        );

        // 被拒和「没有需要推送的」是两回事，不能混成一句话。
        let rejected = GitPushStats {
            rejected: vec![(GitRefNameBuf::from("refs/heads/main"), None)],
            ..Default::default()
        };
        assert_eq!(
            summarize_push(&rejected, &[], "origin"),
            "origin：1 个因远程已变动被拒（先拉取再推）"
        );

        let pushed = GitPushStats {
            pushed: vec![GitRefNameBuf::from("refs/heads/main")],
            ..Default::default()
        };
        assert_eq!(
            summarize_push(&pushed, &["dev（本地有冲突）".to_owned()], "origin"),
            "origin：已推送 1 个 bookmark，跳过 dev（本地有冲突）"
        );
    }

    #[test]
    fn auto_track_bookmarks_only_covers_configured_remotes() {
        // 默认（没配置）不自动跟踪：拉下来的新 bookmark 保持「未跟踪」。
        assert!(
            auto_track_bookmarks(&RemoteSettingsMap::new())
                .unwrap()
                .is_empty()
        );

        let mut settings = RemoteSettingsMap::new();
        settings.insert(
            RemoteNameBuf::from("origin"),
            RemoteSettings {
                auto_track_bookmarks: Some("release-*".to_owned()),
                auto_track_created_bookmarks: None,
                fetch_bookmarks: None,
                fetch_tags: None,
            },
        );

        let matchers = auto_track_bookmarks(&settings).unwrap();
        let matcher = matchers
            .get(&RemoteNameBuf::from("origin"))
            .expect("origin should have an auto-track matcher");
        assert!(matcher.is_match("release-1.0"));
        assert!(!matcher.is_match("feature"));
    }

    #[test]
    fn parse_name_pattern_rejects_garbage() {
        assert!(parse_name_pattern("*").is_ok());
        assert!(parse_name_pattern("release-*").is_ok());
        // 模式语法坏了要报错，而不是当成字面量悄悄用。
        assert!(parse_name_pattern("(").is_err());
    }
}
