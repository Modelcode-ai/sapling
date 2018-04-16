// Copyright (c) 2004-present, Facebook, Inc.
// All Rights Reserved.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2 or any later version.

extern crate bytes;

use std::collections::HashSet;
use std::collections::hash_map::{Entry, HashMap};
use std::fmt::{self, Display};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use futures::{Async, IntoFuture, Poll, Stream};
use futures::future;
use futures::stream;
use futures_ext::{BoxFuture, BoxStream, FutureExt, StreamExt};

use mercurial_types::{fncache_fsencode, simple_fsencode, MPath, MPathElement, RepoPath};
use stockbookmarks::StockBookmarks;
use storage_types::Version;

use blobnode::HgBlobNode;
pub use changeset::RevlogChangeset;
use errors::*;
pub use manifest::RevlogManifest;
use nodehash::{HgChangesetId, HgEntryId, HgManifestId, HgNodeHash, NULL_HASH};
use revlog::{self, RevIdx, Revlog, RevlogIter};

const DEFAULT_LOGS_CAPACITY: usize = 1000000;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Required {
    Store,
    Fncache,
    Dotencode,
    Generaldelta,
    Treemanifest,
    Manifestv2,
    Usefncache,
    Revlogv1,
    Largefiles,
    Lz4revlog,
    SqlDirstate,
    HgSql,
    TreeDirstate,
}

impl Display for Required {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::Required::*;

        let s = match self {
            &Store => "store",
            &Fncache => "fncache",
            &Dotencode => "dotencode",
            &Generaldelta => "generaldelta",
            &Treemanifest => "treemanifest",
            &Manifestv2 => "manifestv2",
            &Usefncache => "usefncache",
            &Revlogv1 => "revlogv1",
            &Largefiles => "largefiles",
            &Lz4revlog => "lz4revlog",
            &SqlDirstate => "sqldirstate",
            &HgSql => "hgsql",
            &TreeDirstate => "treedirstate",
        };
        write!(fmt, "{}", s)
    }
}

impl FromStr for Required {
    type Err = Error;

    fn from_str(s: &str) -> Result<Required> {
        use self::Required::*;

        match s {
            "store" => Ok(Store),
            "fncache" => Ok(Fncache),
            "dotencode" => Ok(Dotencode),
            "generaldelta" => Ok(Generaldelta),
            "treemanifest" => Ok(Treemanifest),
            "manifestv2" => Ok(Manifestv2),
            "usefncache" => Ok(Usefncache),
            "revlogv1" => Ok(Revlogv1),
            "largefiles" => Ok(Largefiles),
            "lz4revlog" => Ok(Lz4revlog),
            "sqldirstate" => Ok(SqlDirstate),
            "hgsql" => Ok(HgSql),
            "treedirstate" => Ok(TreeDirstate),
            unk => Err(ErrorKind::UnknownReq(unk.into()).into()),
        }
    }
}

/// Representation of a whole Mercurial repo
///
/// `Repo` represents a whole repo: ie, the complete history of a set of files.
/// It consists of an overall history in the form of a DAG of revisions, or changesets.
/// This DAG will typically have a single initial version (though it could have more if
/// histories are merged) and one or more heads, which are revisions which have no children.
///
/// Some revisions can be explicitly named with "bookmarks", and they're often heads as well.
///
/// At the filesystem level, the repo consists of:
///  - the changelog: .hg/store/00changelog.[di]
///  - the manifest: .hg/store/00manifest.[di]
///  - the tree manifests: .hg/store/00manifesttree.[di] and .hg/store/meta/.../00manifest.i
///  - per-file histories: .hg/store/data/.../<file>.[di]
#[derive(Debug, Clone)]
pub struct RevlogRepo {
    basepath: PathBuf,               // path to .hg directory
    requirements: HashSet<Required>, // requirements
    changelog: Revlog,               // changes
    inner: Arc<RwLock<RevlogInner>>, // Inner parts
    inmemory_logs_capacity: usize,   // Limit on the number of filelogs and tree revlogs in memory.
                                     // Note: there can be 2 * inmemory_logs_capacity revlogs in
                                     // memory in total: half for filelogs and half for revlogs.
}

pub struct RevlogRepoOptions {
    pub inmemory_logs_capacity: usize,
}

#[derive(Debug)]
struct RevlogInner {
    logcache: HashMap<RepoPath, Revlog>,
}

impl PartialEq<Self> for RevlogRepo {
    fn eq(&self, other: &Self) -> bool {
        self.basepath == other.basepath && self.requirements == other.requirements
            && Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl Eq for RevlogRepo {}

impl RevlogRepo {
    pub fn open<P: Into<PathBuf>>(base: P) -> Result<RevlogRepo> {
        let options = RevlogRepoOptions {
            inmemory_logs_capacity: DEFAULT_LOGS_CAPACITY,
        };
        RevlogRepo::open_with_options(base, options)
    }

    pub fn open_with_options<P: Into<PathBuf>>(
        base: P,
        options: RevlogRepoOptions,
    ) -> Result<RevlogRepo> {
        let base = base.into();
        let store = base.as_path().join("store");

        let changelog =
            Revlog::from_idx_with_data(store.join("00changelog.i"), None as Option<String>)?;

        let mut requirements = HashSet::new();
        let file = fs::File::open(base.join("requires")).context("Can't open `requires`")?;
        for line in BufReader::new(file).lines() {
            requirements.insert(line.context("Line read failed")?.parse()?);
        }

        Ok(RevlogRepo {
            basepath: base.into(),
            requirements,
            changelog,
            inner: Arc::new(RwLock::new(RevlogInner {
                logcache: HashMap::new(),
            })),
            inmemory_logs_capacity: options.inmemory_logs_capacity,
        })
    }

    pub fn get_heads(&self) -> BoxStream<HgNodeHash, Error> {
        match self.changelog.get_heads() {
            Err(e) => stream::once(Err(e)).boxify(),
            Ok(set) => stream::iter_ok(set.into_iter()).boxify(),
        }
    }

    pub fn get_bookmarks(&self) -> Result<StockBookmarks> {
        Ok(StockBookmarks::read(self.basepath.clone())?)
    }

    pub fn get_bookmark_value(
        &self,
        key: &AsRef<[u8]>,
    ) -> BoxFuture<Option<(HgChangesetId, Version)>, Error> {
        match self.get_bookmarks() {
            Ok(b) => b.get(key).boxify(),
            Err(e) => future::err(e).boxify(),
        }
    }

    pub fn changesets(&self) -> ChangesetStream {
        ChangesetStream::new(&self.changelog)
    }

    pub fn get_changeset(&self, changesetid: &HgChangesetId) -> BoxFuture<RevlogChangeset, Error> {
        // TODO: (jsgf) T17932873 distinguish between not existing vs some other error
        let nodeid = changesetid.clone().into_nodehash();
        self.changelog
            .get_idx_by_nodeid(&nodeid)
            .and_then(|idx| self.changelog.get_rev(idx))
            .and_then(|rev| RevlogChangeset::new(rev))
            .into_future()
            .boxify()
    }

    pub fn get_root_manifest(&self, manifestid: &HgManifestId) -> BoxFuture<RevlogManifest, Error> {
        // TODO: (jsgf) T17932873 distinguish between not existing vs some other error
        let nodeid = manifestid.clone().into_nodehash();
        let repo = self.clone();
        let revlog = try_boxfuture!(self.get_path_revlog(&RepoPath::root()));
        revlog
            .get_idx_by_nodeid(&nodeid)
            .and_then(|idx| revlog.get_rev(idx))
            .and_then(move |rev| RevlogManifest::new(repo, rev))
            .into_future()
            .boxify()
    }

    pub fn get_requirements(&self) -> &HashSet<Required> {
        &self.requirements
    }

    /// This method is used by RevlogManifest to traverse the Revlogs in search of manifests and
    /// files. Users of this crate should rely on RevlogManifest traversal or use
    /// RevlogRepo::get_manifest directly.
    pub(crate) fn get_path_revlog(&self, path: &RepoPath) -> Result<Revlog> {
        use mercurial_types::RepoPath::*;

        if let Some(revlog) = self.get_path_revlog_from_cache(path) {
            return Ok(revlog);
        }
        let mut inner = self.inner.write().expect("poisoned lock");

        // We may have memory issues if we are keeping too many revlogs in memory.
        // Let's clear them when we have too much
        if inner.logcache.len() > self.inmemory_logs_capacity {
            inner.logcache.clear();
        }

        match inner.logcache.entry(path.clone()) {
            Entry::Occupied(log) => Ok(log.get().clone()),

            Entry::Vacant(missing) => {
                let revlog_path = match *path {
                    // .hg/store/00manifesttree
                    RootPath => MPath::new("00manifesttree")?,
                    // .hg/store/meta/<path>/00manifest
                    DirectoryPath(_) => MPath::new("meta")?
                        .join(MPath::iter_opt(path.mpath()))
                        .join(&MPath::new("00manifest")?),
                    // .hg/store/data/<path>
                    FilePath(_) => MPath::new("data")?.join(MPath::iter_opt(path.mpath())),
                };
                Ok(missing
                    .insert(self.init_revlog_from_path(revlog_path)?)
                    .clone())
            }
        }
    }

    fn get_path_revlog_from_cache(&self, path: &RepoPath) -> Option<Revlog> {
        let inner = self.inner.read().expect("poisoned lock");
        inner.logcache.get(path).cloned()
    }

    /// path is the path to the revlog files, but without the .i or .d extensions
    fn init_revlog_from_path(&self, path: MPath) -> Result<Revlog> {
        let mut elements: Vec<MPathElement> = path.into_iter().collect();
        let mut basename = elements.pop().ok_or_else(|| {
            format_err!("empty path provided to RevlogRepo::init_revlog_from_path")
        })?;

        let index_path = {
            let mut basename = basename.clone();
            basename.extend(b".i");
            elements.push(basename);
            self.fsencode_path(&elements)
        };
        elements.pop();

        let data_path = {
            basename.extend(b".d");
            elements.push(basename);
            self.fsencode_path(&elements)
        };

        let store_path = self.basepath.join("store");
        Revlog::from_idx_with_data(
            store_path.join(index_path),
            Some(store_path.join(data_path)),
        )
    }

    fn fsencode_path(&self, elements: &[MPathElement]) -> PathBuf {
        // Mercurial has a complicated logic of path encoding.
        // Code below matches core Mercurial logic from the commit
        // 75013952d8d9608f73cd45f68405fbd6ec112bf2 from file mercurial/store.py from the function
        // store(). The only caveat is that basicstore is not yet implemented
        if self.requirements.contains(&Required::Store) {
            if self.requirements.contains(&Required::Fncache) {
                let dotencode = self.requirements.contains(&Required::Dotencode);
                fncache_fsencode(&elements, dotencode)
            } else {
                simple_fsencode(&elements)
            }
        } else {
            unimplemented!();
        }
    }
}

#[deprecated(note = "This is going to be deleted soon. It is used only in blobimport crate")]
pub trait RevlogRepoBlobimportExt {
    fn get_changelog_entry_by_id(&self, id: &HgEntryId) -> Result<revlog::Entry>;

    fn get_changelog_entry_by_idx(&self, revidx: RevIdx) -> Result<revlog::Entry>;

    fn get_manifest_blob_by_id(&self, nodeid: &HgNodeHash) -> Result<HgBlobNode>;

    fn get_path_revlog(&self, path: &RepoPath) -> Result<Revlog>;
}

#[allow(deprecated)]
impl RevlogRepoBlobimportExt for RevlogRepo {
    fn get_changelog_entry_by_id(&self, id: &HgEntryId) -> Result<revlog::Entry> {
        self.changelog.get_entry_by_id(&id)
    }

    fn get_changelog_entry_by_idx(&self, revidx: RevIdx) -> Result<revlog::Entry> {
        self.changelog.get_entry(revidx)
    }

    fn get_manifest_blob_by_id(&self, nodeid: &HgNodeHash) -> Result<HgBlobNode> {
        // It's possible that commit has null pointer to manifest hash.
        // In that case we want to return empty blobnode
        if nodeid == &NULL_HASH {
            Ok(HgBlobNode::new(Bytes::new(), None, None))
        } else {
            let manifest = self.get_path_revlog(&RepoPath::root())?;
            manifest
                .get_idx_by_nodeid(nodeid)
                .and_then(|idx| manifest.get_rev(idx))
        }
    }

    fn get_path_revlog(&self, path: &RepoPath) -> Result<Revlog> {
        RevlogRepo::get_path_revlog(self, path)
    }
}

pub struct ChangesetStream(RevlogIter);

impl ChangesetStream {
    fn new(changelog: &Revlog) -> Self {
        ChangesetStream(changelog.into_iter())
    }
}

impl Stream for ChangesetStream {
    type Item = HgNodeHash;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<HgNodeHash>, Error> {
        match self.0.next() {
            Some((_, e)) => Ok(Async::Ready(Some(e.nodeid))),
            None => Ok(Async::Ready(None)),
        }
    }
}
