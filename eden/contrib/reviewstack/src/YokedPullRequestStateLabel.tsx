/**
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

import type {PullRequestReviewDecision} from './generated/graphql';

import {PullRequestState} from './generated/graphql';
import {pullRequestReviewDecisionLabel} from './utils';
import {ActionMenu, ActionList, StateLabel, Token, StyledOcticon} from '@primer/react';
import {
  GitPullRequestIcon,
  GitMergeIcon,
  GitPullRequestClosedIcon,
  LinkExternalIcon,
} from '@primer/octicons-react';
import {useRecoilCallback, useRecoilValue} from 'recoil';
import {gitHubClient, gitHubPullRequest, gitHubPullRequestViewerDidAuthor} from './recoil';
import useRefreshPullRequest from './useRefreshPullRequest';

type Status = 'pullClosed' | 'pullMerged' | 'pullOpened';

export default function PullRequestStateLabel({
  reviewDecision,
  state,
  variant = 'small',
  plaintext = false,
  url,
}: {
  reviewDecision: PullRequestReviewDecision | null;
  state: PullRequestState;
  variant?: 'small' | 'normal';
  plaintext?: boolean | undefined;
  url: string;
}) {
  const {status, label, color} = statusAndLabel(state, reviewDecision);
  const tagIcon = {
    [PullRequestState.Closed]: GitPullRequestClosedIcon,
    [PullRequestState.Merged]: GitMergeIcon,
    [PullRequestState.Open]: GitPullRequestIcon,
  }[state];
  const refreshPullRequest = useRefreshPullRequest();
  const viewerDidAuthor = useRecoilValue(gitHubPullRequestViewerDidAuthor);
  if (plaintext) {
    return (
      <>
        <StyledOcticon icon={tagIcon} size={12} /> {label}
      </>
    );
  }

  const mergePullRequest = useRecoilCallback<[], Promise<void>>(
    ({snapshot}) =>
      async () => {
        const clientLoadable = snapshot.getLoadable(gitHubClient);
        if (clientLoadable.state !== 'hasValue' || clientLoadable.contents == null) {
          return Promise.reject('client not found');
        }
        const client = clientLoadable.contents;

        const pullRequestId = snapshot.getLoadable(gitHubPullRequest).valueMaybe()?.id;
        if (pullRequestId == null) {
          return Promise.reject('pull request not found');
        }

        await client.mergePullRequest({
          pullRequestId,
        });

        refreshPullRequest();
      },
    [refreshPullRequest],
  );

  return (
    <ActionMenu>
      <ActionMenu.Anchor>
        <Token
          size="large"
          text={label}
          title={`Pull request is ${label.toLowerCase()}.`}
          leadingVisual={() => <StyledOcticon icon={tagIcon} size={16} sx={{marginLeft: '0'}} />}
          sx={{
            color: '#fff',
            backgroundColor: color,
            borderColor: color,
            cursor: 'pointer',
            paddingLeft: '8px',
            paddingRight: '8px',
            userSelect: 'none',
            ':hover': {
              color: '#fff',
              backgroundColor: color,
              boxShadow: 'none',
            },
          }}
        />
      </ActionMenu.Anchor>
      <ActionMenu.Overlay width="small">
        <ActionList selectionVariant="single">
          {state === PullRequestState.Open && viewerDidAuthor ? (
            <ActionList.Item onClick={mergePullRequest}>
              <ActionList.LeadingVisual>
                <StyledOcticon icon={GitMergeIcon} size={16} sx={{marginLeft: '0'}} />
              </ActionList.LeadingVisual>
              Merge pull request
            </ActionList.Item>
          ) : (
            <ActionList.LinkItem href={url} target="_blank">
              <ActionList.LeadingVisual>
                <StyledOcticon icon={LinkExternalIcon} size={16} sx={{marginLeft: '0'}} />
              </ActionList.LeadingVisual>
              View on GitHub
            </ActionList.LinkItem>
          )}
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}

function statusAndLabel(
  state: PullRequestState,
  reviewDecision: PullRequestReviewDecision | null,
): {
  status: Status;
  label: string;
  color?: string;
} {
  switch (state) {
    case PullRequestState.Closed: {
      const status = 'pullClosed';
      if (reviewDecision === null) {
        return {status, label: 'Closed', color: 'danger.fg'};
      }
      const {label, variant} = pullRequestReviewDecisionLabel(reviewDecision);
      return {status, label, color: `${variant}.fg`};
    }

    case PullRequestState.Merged: {
      const status = 'pullMerged';
      if (reviewDecision === null) {
        return {status, label: 'Merged', color: 'done.fg'};
      }
      const {label, variant} = pullRequestReviewDecisionLabel(reviewDecision);
      return {status, label, color: `${variant}.fg`};
    }

    case PullRequestState.Open: {
      const status = 'pullOpened';
      if (reviewDecision === null) {
        return {status, label: 'Open', color: 'success.fg'};
      }
      const {label, variant} = pullRequestReviewDecisionLabel(reviewDecision);
      return {status, label, color: `${variant}.fg`};
    }
  }
}
