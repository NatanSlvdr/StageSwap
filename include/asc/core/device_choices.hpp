#pragma once

#include <algorithm>
#include <cstddef>
#include <span>
#include <string>
#include <vector>

namespace asc {

struct PersistentDeviceChoices {
    // Index zero always represents an explicit "no device" choice. A saved
    // identifier that is temporarily unavailable is appended so displaying
    // and saving a settings form cannot silently erase it.
    std::vector<std::string> identifiers;
    std::size_t selected_index{0};
    bool configured_device_unavailable{false};
};

[[nodiscard]] inline PersistentDeviceChoices build_persistent_device_choices(
    const std::span<const std::string> available_identifiers,
    const std::string& configured_identifier) {
    PersistentDeviceChoices result;
    result.identifiers.reserve(available_identifiers.size() + 2);
    result.identifiers.emplace_back();
    result.identifiers.insert(result.identifiers.end(), available_identifiers.begin(), available_identifiers.end());

    if (configured_identifier.empty()) return result;
    const auto selected = std::find(result.identifiers.begin() + 1, result.identifiers.end(), configured_identifier);
    if (selected != result.identifiers.end()) {
        result.selected_index = static_cast<std::size_t>(std::distance(result.identifiers.begin(), selected));
        return result;
    }

    result.identifiers.push_back(configured_identifier);
    result.selected_index = result.identifiers.size() - 1;
    result.configured_device_unavailable = true;
    return result;
}

} // namespace asc
