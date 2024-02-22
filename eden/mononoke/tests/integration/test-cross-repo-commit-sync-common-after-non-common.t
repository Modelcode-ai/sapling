# Copyright (c) Meta Platforms, Inc. and affiliates.
#
# This software may be used and distributed according to the terms of the
# GNU General Public License found in the LICENSE file in the root
# directory of this source tree.

  $ . "${TEST_FIXTURES}/library.sh"
  $ . "${TEST_FIXTURES}/library-push-redirector.sh"

  $ large_small_megarepo_config
  $ init_large_small_repo
  Adding synced mapping entry
  Starting Mononoke server

-- Show the small and the large repo from the common config (init_large_small_repo, see library-push-redirector.sh)
  $ mononoke_newadmin changelog -R small-mon graph -i $S_B -M
  o  message: first post-move commit
  │
  o  message: pre-move commit

  $ mononoke_newadmin changelog -R large-mon graph -i $L_C -M
  o  message: first post-move commit
  │
  o  message: move commit
  │
  o  message: pre-move commit

-- First, push a non common-pushrebase bookmark (other_bookmark) one commit forward to S_C
  $ testtool_drawdag -R small-mon << EOF
  > S_A-S_B-S_C-S_D
  > # exists: S_A $S_A
  > # exists: S_B $S_B
  > # bookmark: S_C other_bookmark
  > EOF
  S_A=c74140f562eda7c378d4e8d68e4828239617dd51806f3ccb220433a3ea1a6353
  S_B=1ba347e63a4bf200944c22ade8dbea038dd271ef97af346ba4ccfaaefb10dd4d
  S_C=6899eb0af1d64df45683e6bf22c8b82593b22539dec09394f516f944f6fa8c12
  S_D=542a68bb4fd5a7ba5a047a0bb29a48d660c0ea5114688d00b11658313e8f1e6b

-- Then, push a common pushrebase bookmark two commits forward to S_D
  $ mononoke_newadmin bookmarks -R small-mon set master_bookmark $S_D
  Updating publishing bookmark master_bookmark from 1ba347e63a4bf200944c22ade8dbea038dd271ef97af346ba4ccfaaefb10dd4d to 542a68bb4fd5a7ba5a047a0bb29a48d660c0ea5114688d00b11658313e8f1e6b

-- The small repo now looks like this
  $ mononoke_newadmin changelog -R small-mon graph -i $S_D -M
  o  message: S_D
  │
  o  message: S_C
  │
  o  message: first post-move commit
  │
  o  message: pre-move commit

-- Sync after both bookmark moves happened
-- The first bookmark move of other_bookmark to S_C is replicated correctly
-- The second bookmark move of master_bookmark to S_D also suceeds and thanks to the fact that there were no competing pushrebases
-- and the date wasn't rewritten there's no divergence between S_C and S_D.
  $ mononoke_newadmin mutable-counters -R large-mon set xreposync_from_1 0
  Value of xreposync_from_1 in repo large-mon(Id: 0) set to 0
  $ with_stripped_logs mononoke_x_repo_sync 1 0 tail --catch-up-once
  Starting session with id * (glob)
  queue size is 3
  processing log entry #1
  0 unsynced ancestors of 1ba347e63a4bf200944c22ade8dbea038dd271ef97af346ba4ccfaaefb10dd4d
  successful sync bookmark update log #1
  processing log entry #2
  1 unsynced ancestors of 6899eb0af1d64df45683e6bf22c8b82593b22539dec09394f516f944f6fa8c12
  syncing 6899eb0af1d64df45683e6bf22c8b82593b22539dec09394f516f944f6fa8c12
  changeset 6899eb0af1d64df45683e6bf22c8b82593b22539dec09394f516f944f6fa8c12 synced as d06c956180c43660142dabd61da09e9c6d2b19a53f43fee62b5f919789e24411 in * (glob)
  successful sync bookmark update log #2
  processing log entry #3
  2 unsynced ancestors of 542a68bb4fd5a7ba5a047a0bb29a48d660c0ea5114688d00b11658313e8f1e6b
  syncing 6899eb0af1d64df45683e6bf22c8b82593b22539dec09394f516f944f6fa8c12 via pushrebase for master_bookmark
  changeset 6899eb0af1d64df45683e6bf22c8b82593b22539dec09394f516f944f6fa8c12 synced as d06c956180c43660142dabd61da09e9c6d2b19a53f43fee62b5f919789e24411 in * (glob)
  syncing 542a68bb4fd5a7ba5a047a0bb29a48d660c0ea5114688d00b11658313e8f1e6b via pushrebase for master_bookmark
  changeset 542a68bb4fd5a7ba5a047a0bb29a48d660c0ea5114688d00b11658313e8f1e6b synced as 3c072c4093381c801d2a575ccc7943e59ece487b455a5f4781ea7c750af2983e in * (glob)
  successful sync bookmark update log #3

-- Show the bookmarks after the sync
  $ mononoke_newadmin bookmarks --repo-name large-mon list
  d06c956180c43660142dabd61da09e9c6d2b19a53f43fee62b5f919789e24411 bookprefix/other_bookmark
  3c072c4093381c801d2a575ccc7943e59ece487b455a5f4781ea7c750af2983e master_bookmark


-- Show the graph after the sync
  $ mononoke_newadmin changelog -R large-mon graph -i d06c956180c43660142dabd61da09e9c6d2b19a53f43fee62b5f919789e24411,3c072c4093381c801d2a575ccc7943e59ece487b455a5f4781ea7c750af2983e -M
  o  message: S_D
  │
  o  message: S_C
  │
  o  message: first post-move commit
  │
  o  message: move commit
  │
  o  message: pre-move commit
