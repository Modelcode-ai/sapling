/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

use std::io::Write;

use clidispatch::OptionalRepo;
use clidispatch::ReqCtx;
use cmdutil::ConfigSet;
use cmdutil::NoOpts;
use cmdutil::Result;
use configloader::config::Options;

pub fn run(ctx: ReqCtx<NoOpts>, repo: &mut OptionalRepo) -> Result<u8> {
    let mut config = ConfigSet::wrap(repo.config().clone());

    // Set a default repo so we can build valid edenapi URLs outside a repo.
    if matches!(repo, OptionalRepo::None(_)) {
        config.set(
            "remotefilelog",
            "reponame",
            Some("fbsource"),
            &Options::new().source("networkdoctor.rs"),
        );
    }

    let mut stdout = ctx.io().output();
    match network_doctor::Doctor::new().diagnose(repo.config()) {
        Ok(()) => write!(stdout, "No network problems detected.\n")?,
        Err(d) => write!(stdout, "{}\n\n{}\n", d.treatment(repo.config()), d)?,
    };
    Ok(0)
}

pub fn aliases() -> &'static str {
    "debugnetworkdoctor"
}

pub fn doc() -> &'static str {
    "run the (Rust) network doctor"
}

pub fn synopsis() -> Option<&'static str> {
    None
}
