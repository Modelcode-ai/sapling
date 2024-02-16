/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

use std::pin::Pin;

use futures::FutureExt;
use gotham::handler::HandlerFuture;
use gotham::middleware::state::StateMiddleware;
use gotham::pipeline::new_pipeline;
use gotham::pipeline::single_pipeline;
use gotham::prelude::DrawRoutes;
use gotham::router::builder::build_router as gotham_build_router;
use gotham::router::builder::DefineSingleRoute;
use gotham::router::Router;
use gotham::state::State;
use gotham_ext::response::build_response;

use super::error_formatter::GitErrorFormatter;
use crate::model::RepositoryParams;
use crate::model::ServiceType;
use crate::read;

fn capability_advertisement_handler(mut state: State) -> Pin<Box<HandlerFuture>> {
    async move {
        let res = read::capability_advertisement(&mut state).await;
        build_response(res, state, &GitErrorFormatter)
    }
    .boxed()
}
use crate::GitServerContext;

fn health_handler(state: State) -> (State, &'static str) {
    (state, "I_AM_ALIVE\n")
}

pub fn build_router(context: GitServerContext) -> Router {
    let pipeline = new_pipeline().add(StateMiddleware::new(context)).build();

    let (chain, pipelines) = single_pipeline(pipeline);

    gotham_build_router(chain, pipelines, |route| {
        route
            .get("/*repository/info/refs")
            .with_path_extractor::<RepositoryParams>()
            .with_query_string_extractor::<ServiceType>()
            .to(capability_advertisement_handler);

        route.get("/health_check").to(health_handler);
    })
}
