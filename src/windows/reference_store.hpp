#pragma once

#include "common.hpp"
#include "video_frame.hpp"
#include "asc/core/image.hpp"

#include <filesystem>
#include <wincodec.h>

namespace asc::win {

class ReferenceStore {
public:
    ReferenceStore();
    [[nodiscard]] GrayImage load_thumbnail(const std::filesystem::path& path, Size size = {160, 90});
    [[nodiscard]] GrayImage save_frame(const VideoFrame& frame, const std::filesystem::path& path, Size thumbnail_size = {160, 90});
    [[nodiscard]] GrayImage import_image(const std::filesystem::path& source, const std::filesystem::path& destination,
                                         Size thumbnail_size = {160, 90});
private:
    ComPtr<IWICImagingFactory> factory_;
};

} // namespace asc::win
