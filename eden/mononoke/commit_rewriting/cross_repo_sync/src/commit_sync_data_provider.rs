/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

use anyhow::{anyhow, Error};
use bookmark_renaming::{get_bookmark_renamers, BookmarkRenamer, BookmarkRenamers};
use bookmarks::BookmarkName;
use context::CoreContext;
use futures::future::try_join;
use live_commit_sync_config::LiveCommitSyncConfig;
use metaconfig_types::{
    CommitSyncConfig, CommitSyncConfigVersion, CommitSyncDirection, CommonCommitSyncConfig,
};
use mononoke_types::RepositoryId;
use movers::{get_movers, Mover, Movers};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone)]
pub enum CommitSyncDataProvider {
    Live(Arc<dyn LiveCommitSyncConfig>),
}

impl CommitSyncDataProvider {
    pub async fn get_mover(
        &self,
        version: &CommitSyncConfigVersion,
        source_repo_id: RepositoryId,
        target_repo_id: RepositoryId,
    ) -> Result<Mover, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                let commit_sync_config = live_commit_sync_config
                    .get_commit_sync_config_by_version(source_repo_id, version);
                let common_config = live_commit_sync_config.get_common_config(source_repo_id);

                let (commit_sync_config, common_config) =
                    try_join(commit_sync_config, common_config).await?;

                let Movers { mover, .. } = get_movers_from_config(
                    &common_config,
                    &commit_sync_config,
                    source_repo_id,
                    target_repo_id,
                )?;
                Ok(mover)
            }
        }
    }

    pub async fn get_reverse_mover(
        &self,
        version: &CommitSyncConfigVersion,
        source_repo_id: RepositoryId,
        target_repo_id: RepositoryId,
    ) -> Result<Mover, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                let commit_sync_config = live_commit_sync_config
                    .get_commit_sync_config_by_version(source_repo_id, version);
                let common_config = live_commit_sync_config.get_common_config(source_repo_id);

                let (commit_sync_config, common_config) =
                    try_join(commit_sync_config, common_config).await?;

                let Movers { reverse_mover, .. } = get_movers_from_config(
                    &common_config,
                    &commit_sync_config,
                    source_repo_id,
                    target_repo_id,
                )?;
                Ok(reverse_mover)
            }
        }
    }

    pub async fn get_bookmark_renamer(
        &self,
        source_repo_id: RepositoryId,
        target_repo_id: RepositoryId,
    ) -> Result<BookmarkRenamer, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                let commit_sync_config = live_commit_sync_config
                    .get_common_config(source_repo_id)
                    .await?;

                let BookmarkRenamers {
                    bookmark_renamer, ..
                } = get_bookmark_renamers_from_config(
                    &commit_sync_config,
                    source_repo_id,
                    target_repo_id,
                )?;
                Ok(bookmark_renamer)
            }
        }
    }

    pub async fn get_reverse_bookmark_renamer(
        &self,
        source_repo_id: RepositoryId,
        target_repo_id: RepositoryId,
    ) -> Result<BookmarkRenamer, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                let commit_sync_config = live_commit_sync_config
                    .get_common_config(source_repo_id)
                    .await?;

                let BookmarkRenamers {
                    reverse_bookmark_renamer,
                    ..
                } = get_bookmark_renamers_from_config(
                    &commit_sync_config,
                    source_repo_id,
                    target_repo_id,
                )?;
                Ok(reverse_bookmark_renamer)
            }
        }
    }

    pub async fn get_current_version(
        &self,
        ctx: &CoreContext,
        repo_id: RepositoryId,
    ) -> Result<CommitSyncConfigVersion, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                live_commit_sync_config
                    .get_current_commit_sync_config_version(ctx, repo_id)
                    .await
            }
        }
    }

    pub async fn get_small_repos_for_version(
        &self,
        repo_id: RepositoryId,
        version: &CommitSyncConfigVersion,
    ) -> Result<HashSet<RepositoryId>, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                let commit_sync_config = live_commit_sync_config
                    .get_commit_sync_config_by_version(repo_id, &version)
                    .await?;

                Ok(commit_sync_config.small_repos.keys().cloned().collect())
            }
        }
    }

    pub async fn version_exists(
        &self,
        repo_id: RepositoryId,
        version: &CommitSyncConfigVersion,
    ) -> Result<bool, Error> {
        match self {
            Self::Live(live_commit_sync_config) => {
                let versions = live_commit_sync_config
                    .get_all_commit_sync_config_versions(repo_id)
                    .await?;
                Ok(versions.contains_key(version))
            }
        }
    }

    pub async fn get_common_pushrebase_bookmarks(
        &self,
        repo_id: RepositoryId,
    ) -> Result<Vec<BookmarkName>, Error> {
        use CommitSyncDataProvider::*;

        match self {
            Live(live_commit_sync_config) => {
                let common_sync_config = live_commit_sync_config.get_common_config(repo_id).await?;
                Ok(common_sync_config.common_pushrebase_bookmarks)
            }
        }
    }
}

fn get_movers_from_config(
    common_config: &CommonCommitSyncConfig,
    commit_sync_config: &CommitSyncConfig,
    source_repo_id: RepositoryId,
    target_repo_id: RepositoryId,
) -> Result<Movers, Error> {
    let (direction, small_repo_id) =
        get_direction_and_small_repo_id(common_config, source_repo_id, target_repo_id)?;
    get_movers(&commit_sync_config, small_repo_id, direction)
}

fn get_bookmark_renamers_from_config(
    common_config: &CommonCommitSyncConfig,
    source_repo_id: RepositoryId,
    target_repo_id: RepositoryId,
) -> Result<BookmarkRenamers, Error> {
    let (direction, small_repo_id) =
        get_direction_and_small_repo_id(common_config, source_repo_id, target_repo_id)?;
    get_bookmark_renamers(common_config, small_repo_id, direction)
}

fn get_direction_and_small_repo_id(
    common_config: &CommonCommitSyncConfig,
    source_repo_id: RepositoryId,
    target_repo_id: RepositoryId,
) -> Result<(CommitSyncDirection, RepositoryId), Error> {
    let small_repo_id = if common_config.large_repo_id == source_repo_id
        && common_config.small_repos.contains_key(&target_repo_id)
    {
        target_repo_id
    } else if common_config.large_repo_id == target_repo_id
        && common_config.small_repos.contains_key(&source_repo_id)
    {
        source_repo_id
    } else {
        return Err(anyhow!(
            "CommitSyncMapping incompatible with source repo {:?} and target repo {:?}",
            source_repo_id,
            target_repo_id,
        ));
    };

    let direction = if source_repo_id == small_repo_id {
        CommitSyncDirection::SmallToLarge
    } else {
        CommitSyncDirection::LargeToSmall
    };

    Ok((direction, small_repo_id))
}
