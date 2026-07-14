#pragma once

#include "asc/core/config.hpp"
#include "asc/core/controller.hpp"
#include "asc/core/event_log.hpp"

#include <string>
#include <vector>

namespace asc::win {

struct VideoSourcePresentation {
    std::string identifier;
    std::string display_name;
};

struct DashboardPresentation {
    bool manual_override{false};
    bool warning_active{false};
    std::string run_label;
    std::string mode_label;
    std::string output_kind;
    std::string output_name;
    std::string reference_label;
    std::string display_label;
    std::string health_label;
    std::string warning;
    std::string output_tooltip;
    std::string reference_tooltip;
    std::string display_tooltip;
    std::string health_tooltip;
    std::string technical_details;
    std::vector<std::string> recent_activity;
    std::vector<std::string> full_activity;
};

struct DashboardBannerVisibility {
    bool show_warning{false};
    bool show_override{false};
    int row_count{0};
};

inline constexpr Size preview_resolution_cap{640, 360};

[[nodiscard]] DashboardBannerVisibility dashboard_banner_visibility(
    const DashboardPresentation& presentation) noexcept;
[[nodiscard]] Size fit_preview_size(Size source, Size bounds,
                                    Size cap = preview_resolution_cap) noexcept;
[[nodiscard]] std::string unavailable_video_source_status(bool automatic_reconnect);

[[nodiscard]] DashboardPresentation build_dashboard_presentation(
    const AppStatus& status,
    const AppConfig& config,
    const VideoSourcePresentation& video,
    const std::vector<LogEvent>& events,
    TimePoint now = Clock::now());

} // namespace asc::win
