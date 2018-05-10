#!/usr/bin/env python3
#
# Copyright (c) 2016-present, Facebook, Inc.
# All rights reserved.
#
# This source code is licensed under the BSD-style license found in the
# LICENSE file in the root directory of this source tree. An additional grant
# of patent rights can be found in the PATENTS file in the same directory.

import os
import stat
import tempfile

from eden.cli import util

from .lib import testcase


@testcase.eden_repo_test
class RCTest(testcase.EdenRepoTest):

    def populate_repo(self) -> None:
        self.repo.write_file("readme.txt", "test\n")
        self.repo.commit("Initial commit.")

    def test_eden_list(self) -> None:
        mounts = self.eden.list_cmd()
        self.assertEqual({self.mount: self.eden.CLIENT_ACTIVE}, mounts)

        self.eden.unmount(self.mount)
        mounts = self.eden.list_cmd()
        self.assertEqual({}, mounts, msg="There should be 0 mount paths after unmount")

        self.eden.clone(self.repo_name, self.mount)
        mounts = self.eden.list_cmd()
        self.assertEqual({self.mount: self.eden.CLIENT_ACTIVE}, mounts)

    def test_unmount_rmdir(self) -> None:
        clients = os.path.join(self.eden_dir, "clients")
        client_names = os.listdir(clients)
        self.assertEqual(1, len(client_names), msg="There should only be 1 client")
        test_client_dir = os.path.join(clients, client_names[0])

        # Eden list command uses keys of directory map to get mount paths
        mounts = self.eden.list_cmd()
        self.assertEqual({self.mount: self.eden.CLIENT_ACTIVE}, mounts)

        self.eden.unmount(self.mount)
        self.assertFalse(os.path.isdir(test_client_dir))

        # Check that _remove_path_from_directory_map in unmount is successful
        mounts = self.eden.list_cmd()
        self.assertEqual({}, mounts, msg="There should be 0 paths in the directory map")

        self.eden.clone(self.repo_name, self.mount)
        self.assertTrue(
            os.path.isdir(test_client_dir),
            msg="Client name should be restored verbatim because \
                             it should be a function of the mount point",
        )
        mounts = self.eden.list_cmd()
        self.assertEqual(
            {self.mount: self.eden.CLIENT_ACTIVE},
            mounts,
            msg="The client directory should have been restored",
        )

    def test_override_system_config(self) -> None:
        system_repo = self.create_repo("system_repo")

        system_repo.write_file("hello.txt", "hola\n")
        system_repo.commit("Initial commit.")

        repo_info = util.get_repo(system_repo.path)
        assert repo_info is not None

        # Create temporary system config
        f, path = tempfile.mkstemp(dir=self.system_config_dir)

        # Add system_repo to system config file
        config = """\
[repository """ + self.repo_name + """]
path = """ + repo_info.source + """
type = """ + repo_info.type + """
"""
        os.write(f, config.encode("utf-8"))
        os.close(f)

        # Clone repository
        mount_path = os.path.join(self.mounts_dir, self.repo_name + "-1")
        self.eden.clone(self.repo_name, mount_path)

        # Verify that clone used repository data from user config
        readme = os.path.join(mount_path, "hello.txt")
        self.assertFalse(os.path.exists(readme))

        hello = os.path.join(mount_path, "readme.txt")
        st = os.lstat(hello)
        self.assertTrue(stat.S_ISREG(st.st_mode))

        with open(hello, "r") as hello_file:
            self.assertEqual("test\n", hello_file.read())

        # Add system_repo to system config file with new name
        repo_name = "repo"
        f = os.open(path, os.O_WRONLY)
        config = """\
[repository """ + repo_name + """]
path = """ + repo_info.source + """
type = """ + repo_info.type + """
"""
        os.write(f, config.encode("utf-8"))
        os.close(f)

        # Clone repository
        mount_path = os.path.join(self.mounts_dir, repo_name + "-1")
        self.eden.clone(repo_name, mount_path)

        # Verify that clone used repository data from system config
        readme = os.path.join(mount_path, "readme.txt")
        self.assertFalse(os.path.exists(readme))

        hello = os.path.join(mount_path, "hello.txt")
        st = os.lstat(hello)
        self.assertTrue(stat.S_ISREG(st.st_mode))

        with open(hello, "r") as hello_file:
            self.assertEqual("hola\n", hello_file.read())
