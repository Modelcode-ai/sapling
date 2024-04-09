import {useRecoilValue, useRecoilCallback} from 'recoil';
import {gitHubPullRequest, gitHubClient} from './recoil';
import useRefreshPullRequest from './useRefreshPullRequest';

export default function YokedPullRequestMerge() {
  const pullRequest = useRecoilValue(gitHubPullRequest);
  const refreshPullRequest = useRefreshPullRequest();

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
    <button onClick={mergePullRequest} disabled={!pullRequest}>
      Merge Pull Request test
    </button>
  );
}
