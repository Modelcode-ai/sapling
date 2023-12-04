/**
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

import type {Hash} from '../types';
import type {SetLike} from './set';
import type {RecordOf} from 'immutable';

import {HashSet} from './set';
import {Map as ImMap, Record, List} from 'immutable';
import {cached} from 'shared/LRU';
import {SelfUpdate} from 'shared/immutableExt';

/**
 * Hash-map like container with graph related queries.
 *
 * Unlike a traditional source control dag, this works more like a map:
 * - add() does not require parents to (recursively) exist.
 * - remove() does not remove descendants (recursively).
 *
 * The `BaseDag` is a minimal implementation for core graph query
 * needs (like heads, roots, range, ancestors, gca, etc).
 * For use-cases that involve commit attributes, like phases, bookmarks,
 * successors, use `dag/Dag` instead.
 *
 * Internally maintains a "parent -> child" mapping for efficient queries.
 * All queries, regardless of the input size, are O(N) worst case. None
 * should be O(N^2).
 */
export class BaseDag<C extends HashWithParents> extends SelfUpdate<BaseDagRecord<C>> {
  constructor(record?: BaseDagRecord<C>) {
    super(record ?? (EMPTY_DAG_RECORD as BaseDagRecord<C>));
  }

  // Edit

  /**
   * Add commits. Parents do not have to be added first.
   * If a commit with the same hash already exists, it will be replaced.
   */
  add(commits: Iterable<C>): BaseDag<C> {
    const commitArray = [...commits];
    const dag = this.remove(commitArray.map(c => c.hash));
    let {childMap, infoMap} = dag;
    for (const commit of commitArray) {
      commit.parents.forEach(p => {
        const children = childMap.get(p);
        const child = commit.hash;
        const newChildren =
          children == null
            ? List([child])
            : children.contains(child)
            ? children
            : children.push(child);
        childMap = childMap.set(p, newChildren);
      });
      infoMap = infoMap.set(commit.hash, commit);
    }
    const record = dag.inner.merge({infoMap, childMap});
    return new BaseDag(record);
  }

  /** Remove commits by hash. Descendants are not removed automatically. */
  remove(set: SetLike): BaseDag<C> {
    const hashSet = HashSet.fromHashes(set);
    let {childMap, infoMap} = this;
    for (const hash of hashSet) {
      const commit = this.get(hash);
      if (commit == undefined) {
        continue;
      }
      commit.parents.forEach(p => {
        const children = childMap.get(p);
        if (children != null) {
          const newChildren = children.filter(h => h !== hash);
          childMap = childMap.set(p, newChildren);
        }
      });
      infoMap = infoMap.remove(hash);
    }
    const record = this.inner.merge({infoMap, childMap});
    return new BaseDag(record);
  }

  // Basic query

  get(hash: Hash | undefined | null): Readonly<C> | undefined {
    return hash == null ? undefined : this.infoMap.get(hash);
  }

  has(hash: Hash | undefined | null): boolean {
    return this.get(hash) !== undefined;
  }

  [Symbol.iterator](): IterableIterator<Hash> {
    return this.infoMap.keys();
  }

  values(): Iterable<Readonly<C>> {
    return this.infoMap.values();
  }

  /** Get parent hashes. Only return hashes present in this.infoMap. */
  parentHashes(hash: Hash): Readonly<Hash[]> {
    return this.infoMap.get(hash)?.parents?.filter(p => this.infoMap.has(p)) ?? [];
  }

  /** Get child hashes. Only return hashes present in this.infoMap. */
  childHashes(hash: Hash): List<Hash> {
    if (!this.infoMap.has(hash)) {
      return EMPTY_LIST;
    }
    return this.childMap.get(hash) ?? EMPTY_LIST;
  }

  // High-level query

  parents(set: SetLike): HashSet {
    return flatMap(set, h => this.parentHashes(h));
  }

  children(set: SetLike): HashSet {
    return flatMap(set, h => this.childHashes(h));
  }

  /**
   * set + parents(set) + parents(parents(set)) + ...
   * If `within` is set, change `parents` to only return hashes within `within`.
   */
  @cached({cacheSize: 500})
  ancestors(set: SetLike, props?: {within?: SetLike}): HashSet {
    const filter = nullableWithinContains(props?.within);
    return unionFlatMap(set, h => this.parentHashes(h).filter(filter));
  }

  /**
   * set + children(set) + children(children(set)) + ...
   * If `within` is set, change `children` to only return hashes within `within`.
   */
  descendants(set: SetLike, props?: {within?: SetLike}): HashSet {
    const filter = nullableWithinContains(props?.within);
    return unionFlatMap(set, h => this.childHashes(h).filter(filter));
  }

  /** ancestors(heads) & descendants(roots) */
  range(roots: SetLike, heads: SetLike): HashSet {
    // PERF: This is not the most efficient, but easy to write.
    return this.ancestors(heads).intersect(this.descendants(roots));
  }

  /** set - children(set) */
  roots(set: SetLike): HashSet {
    const children = this.children(set);
    return HashSet.fromHashes(set).subtract(children);
  }

  /** set - parents(set) */
  heads(set: SetLike): HashSet {
    const parents = this.parents(set);
    return HashSet.fromHashes(set).subtract(parents);
  }

  /** Greatest common ancestor. heads(ancestors(set1) & ancestors(set2)). */
  gca(set1: SetLike, set2: SetLike): HashSet {
    return this.heads(this.ancestors(set1).intersect(this.ancestors(set2)));
  }

  /** ancestor in ancestors(descendant) */
  isAncestor(ancestor: Hash, descendant: Hash): boolean {
    // PERF: This is not the most efficient, but easy to write.
    return this.ancestors(descendant).contains(ancestor);
  }

  /**
   * Return commits that match the given condition.
   * This can be useful for things like "obsolete()".
   * `set`, if not undefined, limits the search space.
   */
  filter(predicate: (commit: Readonly<C>) => boolean, set?: SetLike): HashSet {
    let hashes: SetLike;
    if (set === undefined) {
      hashes = this.infoMap.filter((commit, _hash) => predicate(commit)).keys();
    } else {
      hashes = HashSet.fromHashes(set)
        .toHashes()
        .filter(h => {
          const c = this.get(h);
          return c != undefined && predicate(c);
        });
    }
    return HashSet.fromHashes(hashes);
  }

  // Delegates

  get infoMap(): ImMap<Hash, Readonly<C>> {
    return this.inner.infoMap;
  }

  get childMap(): ImMap<Hash, List<Hash>> {
    return this.inner.childMap;
  }

  // Filters.

  merge(set?: SetLike): HashSet {
    return this.filter(c => c.parents.length > 1, set);
  }
}

function flatMap(set: SetLike, f: (h: Hash) => List<Hash> | Readonly<Array<Hash>>): HashSet {
  return new HashSet(
    HashSet.fromHashes(set)
      .toHashes()
      .flatMap(h => f(h)),
  );
}

/** set + flatMap(set, f) + flatMap(flatMap(set, f), f) + ... */
export function unionFlatMap(
  set: SetLike,
  f: (h: Hash) => List<Hash> | Readonly<Array<Hash>>,
): HashSet {
  let result = new HashSet().toHashes();
  let newHashes = [...HashSet.fromHashes(set)];
  while (newHashes.length > 0) {
    result = result.concat(newHashes);
    const nextNewHashes: Hash[] = [];
    newHashes.forEach(h => {
      f(h).forEach(v => {
        if (!result.contains(v)) {
          nextNewHashes.push(v);
        }
      });
    });
    newHashes = nextNewHashes;
  }
  return HashSet.fromHashes(result);
}

/**
 * If `set` is undefined, return a function that always returns true.
 * Otherwise, return a function that checks whether `set` contains `h`.
 */
function nullableWithinContains(set?: SetLike): (h: Hash) => boolean {
  if (set === undefined) {
    return _h => true;
  } else {
    const hashSet = HashSet.fromHashes(set);
    return h => hashSet.contains(h);
  }
}

/** Minimal fields needed to be used in commit graph structures. */
export interface HashWithParents {
  hash: Hash;
  parents: Hash[];
  // TODO: We might want "ancestors" to express distant parent relationships.
  // However, sl does not yet have a way to expose that information.
}

type BaseDagProps<C extends HashWithParents> = {
  infoMap: ImMap<Hash, Readonly<C>>;
  // childMap is derived from infoMap.
  childMap: ImMap<Hash, List<Hash>>;
};

const BaseDagRecord = Record<BaseDagProps<HashWithParents>>({
  infoMap: ImMap(),
  childMap: ImMap(),
});
type BaseDagRecord<C extends HashWithParents> = RecordOf<BaseDagProps<C>>;

const EMPTY_DAG_RECORD = BaseDagRecord();
const EMPTY_LIST = List<Hash>();