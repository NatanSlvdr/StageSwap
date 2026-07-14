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

[[nodiscard]] DashboardPresentation build_dashboard_presentation(
    const AppStatus& status,
    const AppConfig& config,
    const VideoSourcePresentation& video,
    const std::vector<LogEvent>& events,
    TimePoint now = Clock::now());

} // namespace asc::win
