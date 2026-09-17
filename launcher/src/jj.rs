use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, anyhow};
use jj_lib::backend::CommitId;
use jj_lib::default_backend_factories::{
    default_backend_factories, default_working_copy_factories,
};
use jj_lib::fileset::FilesetAliasesMap;
use jj_lib::op_store::RefTarget;
use jj_lib::repo::Repo as _;
use jj_lib::revset::{
    RevsetAliasesMap, RevsetDiagnostics, RevsetExtensions, RevsetParseContext,
    RevsetWorkspaceContext, SymbolResolver, parse,
};
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

fn load_workspace(path: &Path) -> anyhow::Result<Workspace> {
    let settings = user_settings::create_user_settings(Some(path))?;
    let store_factories = default_backend_factories();
    let working_copy_factories = default_working_copy_factories();

    Workspace::load(&settings, path, &store_factories, &working_copy_factories).map_err(Into::into)
}

pub fn is_repo<P: AsRef<Path>>(path: P) -> bool {
    load_workspace(path.as_ref()).is_ok()
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
}
