/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

#pragma once

#include <folly/Range.h>
#include <folly/Synchronized.h>
#include <sys/types.h>
#include <atomic>
#include <memory>
#include <vector>

#include "eden/common/telemetry/RequestMetricsScope.h"
#include "eden/common/telemetry/TraceBus.h"
#include "eden/common/utils/RefPtr.h"
#include "eden/fs/eden-config.h"
#include "eden/fs/model/Hash.h"
#include "eden/fs/store/BackingStore.h"
#include "eden/fs/store/ImportPriority.h"
#include "eden/fs/store/LocalStore.h"
#include "eden/fs/store/ObjectFetchContext.h"
#include "eden/fs/store/hg/HgBackingStoreOptions.h"
#include "eden/fs/store/hg/HgImportRequestQueue.h"
#include "eden/fs/telemetry/ActivityBuffer.h"
#include "eden/scm/lib/backingstore/include/SaplingNativeBackingStore.h"

namespace facebook::eden {

class BackingStoreLogger;
class ReloadableConfig;
class LocalStore;
class UnboundedQueueExecutor;
class EdenStats;
class HgImportRequest;
class StructuredLogger;
class FaultInjector;
template <typename T>
class RefPtr;
class ObjectFetchContext;
using ObjectFetchContextPtr = RefPtr<ObjectFetchContext>;

struct HgImportTraceEvent : TraceEventBase {
  enum EventType : uint8_t {
    QUEUE,
    START,
    FINISH,
  };

  enum ResourceType : uint8_t {
    BLOB,
    TREE,
    BLOBMETA,
  };

  static HgImportTraceEvent queue(
      uint64_t unique,
      ResourceType resourceType,
      const HgProxyHash& proxyHash,
      ImportPriority::Class priority,
      ObjectFetchContext::Cause cause,
      OptionalProcessId pid) {
    return HgImportTraceEvent{
        unique, QUEUE, resourceType, proxyHash, priority, cause, pid};
  }

  static HgImportTraceEvent start(
      uint64_t unique,
      ResourceType resourceType,
      const HgProxyHash& proxyHash,
      ImportPriority::Class priority,
      ObjectFetchContext::Cause cause,
      OptionalProcessId pid) {
    return HgImportTraceEvent{
        unique, START, resourceType, proxyHash, priority, cause, pid};
  }

  static HgImportTraceEvent finish(
      uint64_t unique,
      ResourceType resourceType,
      const HgProxyHash& proxyHash,
      ImportPriority::Class priority,
      ObjectFetchContext::Cause cause,
      OptionalProcessId pid) {
    return HgImportTraceEvent{
        unique, FINISH, resourceType, proxyHash, priority, cause, pid};
  }

  HgImportTraceEvent(
      uint64_t unique,
      EventType eventType,
      ResourceType resourceType,
      const HgProxyHash& proxyHash,
      ImportPriority::Class priority,
      ObjectFetchContext::Cause cause,
      OptionalProcessId pid);

  // Simple accessor that hides the internal memory representation of paths.
  std::string getPath() const {
    return path.get();
  }

  // Unique per request, but is consistent across the three stages of an import:
  // queue, start, and finish. Used to correlate events to a request.
  uint64_t unique;
  // Always null-terminated, and saves space in the trace event structure.
  // TODO: Replace with a single pointer to a reference-counted string to save 8
  // bytes in this struct.
  std::shared_ptr<char[]> path;
  // The HG manifest node ID.
  Hash20 manifestNodeId;
  EventType eventType;
  ResourceType resourceType;
  ImportPriority::Class importPriority;
  ObjectFetchContext::Cause importCause;
  OptionalProcessId pid;
};

/**
 * An Hg backing store implementation that will put incoming blob/tree import
 * requests into a job queue, then a pool of workers will work on fulfilling
 * these requests via different methods (reading from hgcache, Mononoke,
 * debugimporthelper, etc.).
 */
class HgQueuedBackingStore final : public BackingStore {
 public:
  using ImportRequestsList = std::vector<std::shared_ptr<HgImportRequest>>;
  using SaplingNativeOptions = sapling::SaplingNativeBackingStoreOptions;
  using ImportRequestsMap = std::
      map<sapling::NodeId, std::pair<ImportRequestsList, RequestMetricsScope>>;

  HgQueuedBackingStore(
      AbsolutePathPiece repository,
      std::shared_ptr<LocalStore> localStore,
      EdenStatsPtr stats,
      UnboundedQueueExecutor* serverThreadPool,
      std::shared_ptr<ReloadableConfig> config,
      std::unique_ptr<HgBackingStoreOptions> runtimeOptions,
      std::shared_ptr<StructuredLogger> structuredLogger,
      std::unique_ptr<BackingStoreLogger> logger,
      FaultInjector* FOLLY_NONNULL faultInjector);

  /**
   * Create an HgQueuedBackingStore suitable for use in unit tests. It uses an
   * inline executor to process loaded objects rather than the thread pools used
   * in production Eden.
   */
  HgQueuedBackingStore(
      AbsolutePathPiece repository,
      std::shared_ptr<LocalStore> localStore,
      EdenStatsPtr stats,
      std::shared_ptr<ReloadableConfig> config,
      std::unique_ptr<HgBackingStoreOptions> runtimeOptions,
      std::shared_ptr<StructuredLogger> structuredLogger,
      std::unique_ptr<BackingStoreLogger> logger,
      FaultInjector* FOLLY_NONNULL faultInjector);

  ~HgQueuedBackingStore() override;

  /**
   * Objects that can be imported from Hg
   */
  enum HgImportObject {
    BLOB,
    TREE,
    BLOBMETA,
    BATCHED_BLOB,
    BATCHED_TREE,
    BATCHED_BLOBMETA,
    PREFETCH
  };
  constexpr static std::array<HgImportObject, 7> hgImportObjects{
      HgImportObject::BLOB,
      HgImportObject::TREE,
      HgImportObject::BLOBMETA,
      HgImportObject::BATCHED_BLOB,
      HgImportObject::BATCHED_TREE,
      HgImportObject::BATCHED_BLOBMETA,
      HgImportObject::PREFETCH};

  static folly::StringPiece stringOfHgImportObject(HgImportObject object);

  ActivityBuffer<HgImportTraceEvent>& getActivityBuffer() {
    return activityBuffer_;
  }

  TraceBus<HgImportTraceEvent>& getTraceBus() const {
    return *traceBus_;
  }

  /**
   * Flush any pending writes to disk.
   *
   * As a side effect, this also reloads the current state of Mercurial's
   * cache, picking up any writes done by Mercurial.
   */
  void flush() {
    store_.flush();
  }

  ObjectComparison compareObjectsById(const ObjectId& one, const ObjectId& two)
      override;

  RootId parseRootId(folly::StringPiece rootId) override;
  std::string renderRootId(const RootId& rootId) override;
  ObjectId parseObjectId(folly::StringPiece objectId) override {
    return staticParseObjectId(objectId);
  }
  std::string renderObjectId(const ObjectId& objectId) override {
    return staticRenderObjectId(objectId);
  }

  static ObjectId staticParseObjectId(folly::StringPiece objectId);
  static std::string staticRenderObjectId(const ObjectId& objectId);

  std::optional<Hash20> getManifestNode(const ObjectId& commitId);

  ImmediateFuture<GetRootTreeResult> getRootTree(
      const RootId& rootId,
      const ObjectFetchContextPtr& context) override;
  ImmediateFuture<std::shared_ptr<TreeEntry>> getTreeEntryForObjectId(
      const ObjectId& /* objectId */,
      TreeEntryType /* treeEntryType */,
      const ObjectFetchContextPtr& /* context */) override {
    throw std::domain_error("unimplemented");
  }

  void getTreeBatch(const ImportRequestsList& requests);

  folly::SemiFuture<GetTreeResult> getTree(
      const ObjectId& id,
      const ObjectFetchContextPtr& context) override;
  folly::SemiFuture<GetBlobResult> getBlob(
      const ObjectId& id,
      const ObjectFetchContextPtr& context) override;
  folly::SemiFuture<GetBlobMetaResult> getBlobMetadata(
      const ObjectId& id,
      const ObjectFetchContextPtr& context) override;

  /**
   * Reads blob metadata from hg cache.
   */
  folly::Try<BlobMetadataPtr> getLocalBlobMetadata(const HgProxyHash& id);

  // Get blob step functions
  folly::SemiFuture<BlobPtr> retryGetBlob(HgProxyHash hgInfo);

  /**
   * Import the manifest for the specified revision using mercurial
   * treemanifest data.
   */
  folly::Future<TreePtr> importTreeManifest(
      const ObjectId& commitId,
      const ObjectFetchContextPtr& context);

  FOLLY_NODISCARD virtual folly::SemiFuture<folly::Unit> prefetchBlobs(
      ObjectIdRange ids,
      const ObjectFetchContextPtr& context) override;

  /**
   * calculates `metric` for `object` imports that are `stage`.
   *    ex. HgQueuedBackingStore::getImportMetrics(
   *          RequestMetricsScope::HgImportStage::PENDING,
   *          HgQueuedBackingStore::HgImportObject::BLOB,
   *          RequestMetricsScope::Metric::COUNT,
   *        )
   *    calculates the number of blob imports that are pending
   */
  size_t getImportMetric(
      RequestMetricsScope::RequestStage stage,
      HgImportObject object,
      RequestMetricsScope::RequestMetric metric) const;

  void startRecordingFetch() override;
  std::unordered_set<std::string> stopRecordingFetch() override;

  ImmediateFuture<folly::Unit> importManifestForRoot(
      const RootId& rootId,
      const Hash20& manifestId,
      const ObjectFetchContextPtr& context) override;

  void periodicManagementTask() override;

  std::optional<folly::StringPiece> getRepoName() override {
    return store_.getRepoName();
  }

  int64_t dropAllPendingRequestsFromQueue() override;

 private:
  // Forbidden copy constructor and assignment operator
  HgQueuedBackingStore(const HgQueuedBackingStore&) = delete;
  HgQueuedBackingStore& operator=(const HgQueuedBackingStore&) = delete;

  folly::Future<TreePtr> importTreeManifestImpl(
      Hash20 manifestNode,
      const ObjectFetchContextPtr& context);

  folly::Try<TreePtr> getTreeFromBackingStore(
      const RelativePath& path,
      const Hash20& manifestId,
      const ObjectId& edenTreeId,
      const ObjectFetchContextPtr& context);

  folly::Future<TreePtr> retryGetTree(
      const Hash20& manifestNode,
      const ObjectId& edenTreeID,
      RelativePathPiece path);

  folly::Future<TreePtr> retryGetTreeImpl(
      Hash20 manifestNode,
      ObjectId edenTreeID,
      RelativePath path,
      std::shared_ptr<LocalStore::WriteBatch> writeBatch);

  void processBlobImportRequests(
      std::vector<std::shared_ptr<HgImportRequest>>&& requests);
  void processTreeImportRequests(
      std::vector<std::shared_ptr<HgImportRequest>>&& requests);
  void processBlobMetaImportRequests(
      std::vector<std::shared_ptr<HgImportRequest>>&& requests);

  /**
   * Import multiple blobs at once. The vector parameters have to be the same
   * length. Promises passed in will be resolved if a blob is successfully
   * imported. Otherwise the promise will be left untouched.
   */
  void getBlobBatch(const ImportRequestsList& requests);

  /**
   * Fetch multiple aux data at once.
   *
   * This function returns when all the aux data have been fetched.
   */
  void getBlobMetadataBatch(const ImportRequestsList& requests);

  /**
   * The worker runloop function.
   */
  void processRequest();

  void logMissingProxyHash();

  /**
   * Fetch a blob from Mercurial.
   *
   * For latency sensitive context, the caller is responsible for checking if
   * the blob is present locally, as this function will always push the request
   * at the end of the queue.
   */
  ImmediateFuture<GetBlobResult> getBlobImpl(
      const ObjectId& id,
      const HgProxyHash& proxyHash,
      const ObjectFetchContextPtr& context);

  /**
   * Imports the blob identified by the given hash from the backing store.
   * If localOnly is set to true, only fetch the blob from local (memory or
   * disk) store.
   *
   * Returns nullptr if not found.
   */
  folly::Try<BlobPtr> getBlobFromBackingStore(
      const HgProxyHash& hgInfo,
      sapling::FetchMode fetchMode);

 public:
  /**
   * Fetch the blob metadata from Mercurial.
   *
   * For latency sensitive context, the caller is responsible for checking if
   * the blob metadata is present locally, as this function will always push
   * the request at the end of the queue.
   *
   * This is marked as public but don't be fooled, this is not intended to be
   * used by anybody but HgQueuedBackingStore and debugGetBlobMetadata Thrift
   * handler.
   */
  ImmediateFuture<GetBlobMetaResult> getBlobMetadataImpl(
      const ObjectId& id,
      const HgProxyHash& proxyHash,
      const ObjectFetchContextPtr& context);

  /**
   * Imports the blob identified by the given hash from the local store.
   * Returns nullptr if not found.
   */
  folly::Try<BlobPtr> getBlobLocal(const HgProxyHash& hgInfo) {
    return getBlobFromBackingStore(hgInfo, sapling::FetchMode::LocalOnly);
  }

  /**
   * Imports the blob identified by the given hash from the remote store.
   * Returns nullptr if not found.
   */
  folly::Try<BlobPtr> getBlobRemote(const HgProxyHash& hgInfo) {
    return getBlobFromBackingStore(hgInfo, sapling::FetchMode::RemoteOnly);
  }

 private:
  /**
   * Fetch a tree from Mercurial.
   *
   * For latency sensitive context, the caller is responsible for checking if
   * the tree is present locally, as this function will always push the request
   * at the end of the queue.
   */
  ImmediateFuture<GetTreeResult> getTreeEnqueue(
      const ObjectId& id,
      const HgProxyHash& proxyHash,
      const ObjectFetchContextPtr& context);

  /**
   * Logs a backing store fetch to scuba if the path being fetched is in the
   * configured paths to log. The path is derived from the proxy hash.
   */
  void logBackingStoreFetch(
      const ObjectFetchContext& context,
      folly::Range<HgProxyHash*> hashes,
      ObjectFetchContext::ObjectType type);

  /**
   * gets the watches timing `object` imports that are `stage`
   *    ex. HgQueuedBackingStore::getImportWatches(
   *          RequestMetricsScope::HgImportStage::PENDING,
   *          HgQueuedBackingStore::HgImportObject::BLOB,
   *        )
   *    gets the watches timing blob imports that are pending
   */
  RequestMetricsScope::LockedRequestWatchList& getImportWatches(
      RequestMetricsScope::RequestStage stage,
      HgImportObject object) const;

  /**
   * Gets the watches timing pending `object` imports
   *   ex. HgQueuedBackingStore::getPendingImportWatches(
   *          HgQueuedBackingStore::HgImportObject::BLOB,
   *        )
   *    gets the watches timing pending blob imports
   */
  RequestMetricsScope::LockedRequestWatchList& getPendingImportWatches(
      HgImportObject object) const;

  /**
   * Gets the watches timing live `object` imports
   *   ex. HgQueuedBackingStore::getLiveImportWatches(
   *          HgQueuedBackingStore::HgImportObject::BLOB,
   *        )
   *    gets the watches timing live blob imports
   */
  RequestMetricsScope::LockedRequestWatchList& getLiveImportWatches(
      HgImportObject object) const;

  template <typename T>
  std::pair<ImportRequestsMap, std::vector<sapling::NodeId>> prepareRequests(
      const ImportRequestsList& importRequests,
      const std::string& requestType);

  /**
   * Imports the tree identified by the given hash from the local store.
   * Returns nullptr if not found.
   */
  TreePtr getTreeLocal(
      const ObjectId& edenTreeId,
      const HgProxyHash& proxyHash);

  /**
   * Imports the tree identified by the given hash from the remote store.
   * Returns nullptr if not found.
   */
  folly::Try<TreePtr> getTreeRemote(
      const RelativePath& path,
      const Hash20& manifestId,
      const ObjectId& edenTreeId,
      const ObjectFetchContextPtr& context);

  /**
   * isRecordingFetch_ indicates if HgQueuedBackingStore is recording paths
   * for fetched files. Initially we don't record paths. When
   * startRecordingFetch() is called, isRecordingFetch_ is set to true and
   * recordFetch() will record the input path. When stopRecordingFetch() is
   * called, isRecordingFetch_ is set to false and recordFetch() no longer
   * records the input path.
   */
  std::atomic<bool> isRecordingFetch_{false};
  folly::Synchronized<std::unordered_set<std::string>> fetchedFilePaths_;

  std::shared_ptr<LocalStore> localStore_;
  EdenStatsPtr stats_;

  // A set of threads processing Sapling retry requests.
  std::unique_ptr<folly::Executor> retryThreadPool_;

  /**
   * Reference to the eden config, may be a null pointer in unit tests.
   */
  std::shared_ptr<ReloadableConfig> config_;

  // The main server thread pool; we push the Futures back into
  // this pool to run their completion code to avoid clogging
  // the importer pool. Queuing in this pool can never block (which would risk
  // deadlock) or throw an exception when full (which would incorrectly fail the
  // load).
  folly::Executor* serverThreadPool_;

  /**
   * The import request queue. This queue is unbounded. This queue
   * implementation will ensure enqueue operation never blocks.
   */
  HgImportRequestQueue queue_;

  /**
   * The worker thread pool. These threads will be running `processRequest`
   * forever to process incoming import requests
   */
  std::vector<std::thread> threads_;

  std::shared_ptr<StructuredLogger> structuredLogger_;

  /**
   * Logger for backing store imports
   */
  std::unique_ptr<BackingStoreLogger> logger_;

  FaultInjector& faultInjector_;

  // The last time we logged a missing proxy hash so the minimum interval is
  // limited to EdenConfig::missingHgProxyHashLogInterval.
  folly::Synchronized<std::chrono::steady_clock::time_point>
      lastMissingProxyHashLog_;

  // Track metrics for queued imports
  mutable RequestMetricsScope::LockedRequestWatchList pendingImportBlobWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList
      pendingImportBlobMetaWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList pendingImportTreeWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList
      pendingImportPrefetchWatches_;

  // Track metrics for imports currently fetching data from hg
  mutable RequestMetricsScope::LockedRequestWatchList liveImportBlobWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList liveImportTreeWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList
      liveImportBlobMetaWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList
      liveImportPrefetchWatches_;

  // Track metrics for the number of live batches
  mutable RequestMetricsScope::LockedRequestWatchList liveBatchedBlobWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList liveBatchedTreeWatches_;
  mutable RequestMetricsScope::LockedRequestWatchList
      liveBatchedBlobMetaWatches_;

  std::unique_ptr<HgBackingStoreOptions> runtimeOptions_;

  ActivityBuffer<HgImportTraceEvent> activityBuffer_;

  // The traceBus_ and hgTraceHandle_ should be last so any internal subscribers
  // can capture [this].
  std::shared_ptr<TraceBus<HgImportTraceEvent>> traceBus_;

  // Handle for TraceBus subscription.
  TraceSubscriptionHandle<HgImportTraceEvent> hgTraceHandle_;

  sapling::SaplingNativeBackingStore store_;
};

} // namespace facebook::eden
