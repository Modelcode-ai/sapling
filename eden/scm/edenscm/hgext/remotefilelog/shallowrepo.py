# Copyright (c) Facebook, Inc. and its affiliates.
#
# This software may be used and distributed according to the terms of the
# GNU General Public License version 2.

# shallowrepo.py - shallow repository that uses remote filelogs
from __future__ import absolute_import

import os

from edenscm.mercurial import encoding, error, localrepo, match, progress, scmutil, util
from edenscm.mercurial.i18n import _
from edenscm.mercurial.node import hex, nullid, nullrev
from edenscm.mercurial.pycompat import iteritems

from . import constants, fileserverclient, remotefilectx, remotefilelog, shallowutil
from .repack import domaintenancerepack


requirement = "remotefilelog"


def wraprepo(repo):
    class shallowrepository(repo.__class__):
        @util.propertycache
        def name(self):
            return self.ui.config("remotefilelog", "reponame", "unknown")

        @util.propertycache
        def fallbackpath(self):
            path = self.ui.config(
                "remotefilelog",
                "fallbackpath",
                # fallbackrepo is the old, deprecated name
                self.ui.config(
                    "remotefilelog", "fallbackrepo", self.ui.config("paths", "default")
                ),
            )
            if not path:
                raise error.Abort(
                    "no remotefilelog server " "configured - is your .hg/hgrc trusted?"
                )

            return path

        @util.propertycache
        def fileslog(self):
            return remotefilelog.remotefileslog(self)

        def maybesparsematch(self, *revs, **kwargs):
            """
            A wrapper that allows the remotefilelog to invoke sparsematch() if
            this is a sparse repository, or returns None if this is not a
            sparse repository.
            """
            if util.safehasattr(self, "sparsematch"):
                return self.sparsematch(*revs, **kwargs)

            return None

        def file(self, f):
            if f[0] == "/":
                f = f[1:]

            if self.shallowmatch(f):
                return remotefilelog.remotefilelog(self.svfs, f, self)
            else:
                return super(shallowrepository, self).file(f)

        def filectx(self, path, changeid=None, fileid=None):
            if self.shallowmatch(path):
                return remotefilectx.remotefilectx(self, path, changeid, fileid)
            else:
                return super(shallowrepository, self).filectx(path, changeid, fileid)

        def close(self):
            result = super(shallowrepository, self).close()
            if "fileslog" in self.__dict__:
                self.fileslog.abortpending()
            return result

        def commitpending(self):
            super(shallowrepository, self).commitpending()

            self.numtransactioncommits += 1
            # In some cases, we can have many transactions in the same repo, in
            # which case each one will create a packfile, let's trigger a repack at
            # this point to bring the number of packfiles down to a reasonable
            # number.
            if self.numtransactioncommits >= self.ui.configint(
                "remotefilelog", "commitsperrepack"
            ):
                domaintenancerepack(self)
                self.numtransactioncommits = 0

        def commitctx(self, ctx, error=False):
            """Add a new revision to current repository.
            Revision information is passed via the context argument.
            """

            # some contexts already have manifest nodes, they don't need any
            # prefetching (for example if we're just editing a commit message
            # we can reuse manifest
            if not ctx.manifestnode():
                # prefetch files that will likely be compared
                m1 = ctx.p1().manifest()
                files = []
                for f in ctx.modified() + ctx.added():
                    fparent1 = m1.get(f, nullid)
                    if fparent1 != nullid:
                        files.append((f, hex(fparent1)))
                self.fileservice.prefetch(files)
            return super(shallowrepository, self).commitctx(ctx, error=error)

        def backgroundprefetch(
            self, revs, base=None, repack=False, pats=None, opts=None
        ):
            """Runs prefetch in background with optional repack
            """
            cmd = [util.hgexecutable(), "-R", self.origroot, "prefetch"]
            if repack:
                cmd.append("--repack")
            if revs:
                cmd += ["-r", revs]
            if base:
                cmd += ["-b", base]

            util.spawndetached(cmd)

        def prefetch(self, revs, base=None, pats=None, opts=None, matcher=None):
            """Prefetches all the necessary file revisions for the given revs
            Optionally runs repack in background
            """
            with self._lock(
                self.svfs,
                "prefetchlock",
                True,
                None,
                None,
                _("prefetching in %s") % self.origroot,
            ):
                self._prefetch(revs, base, pats, opts, matcher)

        def _prefetch(self, revs, base=None, pats=None, opts=None, matcher=None):
            fallbackpath = self.fallbackpath
            if fallbackpath:
                # If we know a rev is on the server, we should fetch the server
                # version of those files, since our local file versions might
                # become obsolete if the local commits are stripped.
                localrevs = self.revs("draft()")
                if base is not None and base != nullrev:
                    serverbase = list(
                        self.revs("first(reverse(::%s) - %ld)", base, localrevs)
                    )
                    if serverbase:
                        base = serverbase[0]
            else:
                localrevs = self

            mfl = self.manifestlog
            if base is not None:
                mfdict = mfl[self[base].manifestnode()].read()
                skip = set(iteritems(mfdict))
            else:
                skip = set()

            # Copy the skip set to start large and avoid constant resizing,
            # and since it's likely to be very similar to the prefetch set.
            files = skip.copy()
            serverfiles = skip.copy()
            visited = set()
            visited.add(nullid)
            with progress.bar(self.ui, _("prefetching"), total=len(revs)) as prog:
                for rev in sorted(revs):
                    ctx = self[rev]
                    if pats:
                        m = scmutil.match(ctx, pats, opts)
                    elif matcher is None:
                        matcher = self.maybesparsematch(rev)

                    mfnode = ctx.manifestnode()
                    mfctx = mfl[mfnode]

                    mf = mfctx.read()

                    diff = []
                    if pats:
                        diff.extend(iteritems(mf.matches(m)))
                    if matcher:
                        diff.extend(iteritems(mf.matches(matcher)))
                    if not pats and not matcher:
                        diff.extend(iteritems(mf))
                    if rev not in localrevs:
                        serverfiles.update(diff)
                    else:
                        files.update(diff)

                    visited.add(mfctx.node())
                    prog.value += 1

            files.difference_update(skip)
            serverfiles.difference_update(skip)

            # Fetch files known to be on the server
            if serverfiles:
                results = [(path, hex(fnode)) for (path, fnode) in serverfiles]
                self.fileservice.prefetch(results, force=True)

            # Fetch files that may or may not be on the server
            if files:
                results = [(path, hex(fnode)) for (path, fnode) in files]
                self.fileservice.prefetch(results)

    repo.__class__ = shallowrepository

    repo.shallowmatch = match.always(repo.root, "")
    repo.fileservice = fileserverclient.fileserverclient(repo)

    repo.numtransactioncommits = 0

    repo.includepattern = repo.ui.configlist("remotefilelog", "includepattern", None)
    repo.excludepattern = repo.ui.configlist("remotefilelog", "excludepattern", None)

    if repo.includepattern or repo.excludepattern:
        repo.shallowmatch = match.match(
            repo.root, "", None, repo.includepattern, repo.excludepattern
        )
