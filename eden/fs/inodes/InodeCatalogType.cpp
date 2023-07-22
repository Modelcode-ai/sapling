/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

#include "eden/fs/inodes/InodeCatalogType.h"

namespace facebook::eden {

namespace {

constexpr auto inodeCatalogTypeStr = [] {
  std::array<folly::StringPiece, 8> mapping{};
  mapping[folly::to_underlying(InodeCatalogType::Legacy)] = "Legacy";
  mapping[folly::to_underlying(InodeCatalogType::Sqlite)] = "Sqlite";
  mapping[folly::to_underlying(InodeCatalogType::SqliteInMemory)] =
      "SqliteInMemory";
  mapping[folly::to_underlying(InodeCatalogType::SqliteSynchronousOff)] =
      "SqliteSynchronousOff";
  mapping[folly::to_underlying(InodeCatalogType::SqliteBuffered)] =
      "SqliteBuffered";
  mapping[folly::to_underlying(InodeCatalogType::SqliteInMemoryBuffered)] =
      "SqliteInMemoryBuffered";
  mapping[folly::to_underlying(
      InodeCatalogType::SqliteSynchronousOffBuffered)] =
      "SqliteSynchronousOffBuffered";
  mapping[folly::to_underlying(InodeCatalogType::InMemory)] = "InMemory";
  return mapping;
}();

}

folly::Expected<InodeCatalogType, std::string>
FieldConverter<InodeCatalogType>::fromString(
    folly::StringPiece value,
    const std::map<std::string, std::string>& /*unused*/) const {
  for (auto type = 0ul; type < inodeCatalogTypeStr.size(); type++) {
    if (value.equals(
            inodeCatalogTypeStr[type], folly::AsciiCaseInsensitive())) {
      return static_cast<InodeCatalogType>(type);
    }
  }

  return folly::makeUnexpected(fmt::format(
      "Failed to convert value '{}' to a InodeCatalogType.", value));
}

std::string FieldConverter<InodeCatalogType>::toDebugString(
    InodeCatalogType value) const {
  return inodeCatalogTypeStr[folly::to_underlying(value)].str();
}

} // namespace facebook::eden
