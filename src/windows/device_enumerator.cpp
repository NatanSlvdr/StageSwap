#include "device_enumerator.hpp"
#include "common.hpp"

#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include <physicalmonitorenumerationapi.h>
#include <vector>

namespace asc::win {
namespace {
constexpr DWORD first_video_stream = static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM);

std::string guid_text(const GUID& guid) {
    wchar_t text[64]{};
    StringFromGUID2(guid, text, ARRAYSIZE(text));
    return utf8(text);
}

void enrich_display_identity(const std::wstring_view gdi_name, asc::MonitorIdentity& identity) {
    UINT32 path_count = 0, mode_count = 0;
    if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &path_count, &mode_count) != ERROR_SUCCESS) return;
    std::vector<DISPLAYCONFIG_PATH_INFO> paths(path_count);
    std::vector<DISPLAYCONFIG_MODE_INFO> modes(mode_count);
    if (QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &path_count, paths.data(), &mode_count, modes.data(), nullptr) != ERROR_SUCCESS) return;
    paths.resize(path_count);
    for (const auto& path : paths) {
        DISPLAYCONFIG_SOURCE_DEVICE_NAME source{};
        source.header = {DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, sizeof(source), path.sourceInfo.adapterId, path.sourceInfo.id};
        if (DisplayConfigGetDeviceInfo(&source.header) != ERROR_SUCCESS || gdi_name != source.viewGdiDeviceName) continue;
        DISPLAYCONFIG_TARGET_DEVICE_NAME target{};
        target.header = {DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, sizeof(target), path.targetInfo.adapterId, path.targetInfo.id};
        if (DisplayConfigGetDeviceInfo(&target.header) == ERROR_SUCCESS) {
            identity.device_path = utf8(target.monitorDevicePath);
            identity.hardware_id = identity.device_path;
            identity.manufacturer = std::to_string(target.edidManufactureId);
            if (target.monitorFriendlyDeviceName[0]) identity.model = utf8(target.monitorFriendlyDeviceName);
            identity.adapter_id = std::to_string(path.targetInfo.adapterId.HighPart) + ":" + std::to_string(path.targetInfo.adapterId.LowPart);
        }
        if (path.targetInfo.refreshRate.Denominator)
            identity.refresh_rate_millihz = static_cast<std::uint32_t>((static_cast<std::uint64_t>(path.targetInfo.refreshRate.Numerator) * 1000) / path.targetInfo.refreshRate.Denominator);
        return;
    }
}

BOOL CALLBACK monitor_callback(HMONITOR handle, HDC, RECT*, LPARAM context) {
    auto& output = *reinterpret_cast<std::vector<MonitorDevice>*>(context);
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    if (!GetMonitorInfoW(handle, &info)) return TRUE;
    DEVMODEW mode{};
    mode.dmSize = sizeof(mode);
    EnumDisplaySettingsExW(info.szDevice, ENUM_CURRENT_SETTINGS, &mode, 0);
    DISPLAY_DEVICEW adapter{};
    adapter.cb = sizeof(adapter);
    EnumDisplayDevicesW(info.szDevice, 0, &adapter, EDD_GET_DEVICE_INTERFACE_NAME);
    DISPLAY_DEVICEW panel{};
    panel.cb = sizeof(panel);
    EnumDisplayDevicesW(info.szDevice, 0, &panel, EDD_GET_DEVICE_INTERFACE_NAME);
    asc::MonitorIdentity identity;
    identity.device_path = utf8(panel.DeviceID[0] ? panel.DeviceID : adapter.DeviceID);
    identity.hardware_id = utf8(panel.DeviceKey[0] ? panel.DeviceKey : adapter.DeviceKey);
    identity.adapter_id = utf8(adapter.DeviceID);
    identity.model = utf8(panel.DeviceString[0] ? panel.DeviceString : adapter.DeviceString);
    identity.resolution = {mode.dmPelsWidth, mode.dmPelsHeight};
    identity.orientation_degrees = mode.dmDisplayOrientation * 90;
    identity.refresh_rate_millihz = mode.dmDisplayFrequency * 1000;
    identity.desktop_x = mode.dmPosition.x;
    identity.desktop_y = mode.dmPosition.y;
    enrich_display_identity(info.szDevice, identity);
    output.push_back({handle, std::move(identity), info.szDevice});
    return TRUE;
}
}

std::vector<VideoDevice> enumerate_video_devices() {
    ComPtr<IMFAttributes> attributes;
    check_hresult(MFCreateAttributes(&attributes, 1), "MFCreateAttributes");
    check_hresult(attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID), "Set capture source type");
    IMFActivate** raw = nullptr;
    UINT32 count = 0;
    check_hresult(MFEnumDeviceSources(attributes.Get(), &raw, &count), "MFEnumDeviceSources");
    std::vector<VideoDevice> result;
    result.reserve(count);
    for (UINT32 i = 0; i < count; ++i) {
        ComPtr<IMFActivate> activate;
        activate.Attach(raw[i]);
        CoTaskMemString name;
        UINT32 name_length = 0;
        activate->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name.value, &name_length);
        CoTaskMemString id;
        UINT32 id_length = 0;
        activate->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &id.value, &id_length);
        if (name.value && std::wstring_view(name.value).find(L"Automatic Screen Camera") != std::wstring_view::npos) continue;
        VideoDevice device{name.value ? utf8(name.value) : "Unnamed video device", id.value ? utf8(id.value) : "", {}, true};
        ComPtr<IMFMediaSource> source;
        if (SUCCEEDED(activate->ActivateObject(IID_PPV_ARGS(&source)))) {
            ComPtr<IMFSourceReader> reader;
            if (SUCCEEDED(MFCreateSourceReaderFromMediaSource(source.Get(), nullptr, &reader))) {
                for (DWORD type_index = 0;; ++type_index) {
                    ComPtr<IMFMediaType> type;
                    const auto type_result = reader->GetNativeMediaType(first_video_stream, type_index, &type);
                    if (type_result == MF_E_NO_MORE_TYPES) break;
                    if (FAILED(type_result)) break;
                    UINT32 width = 0, height = 0, numerator = 0, denominator = 1;
                    GUID subtype{};
                    if (SUCCEEDED(MFGetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, &width, &height)) &&
                        SUCCEEDED(MFGetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, &numerator, &denominator)) &&
                        SUCCEEDED(type->GetGUID(MF_MT_SUBTYPE, &subtype)))
                        device.formats.push_back({{width, height}, numerator, denominator, guid_text(subtype)});
                }
            }
            source->Shutdown();
        }
        result.push_back(std::move(device));
    }
    CoTaskMemFree(raw);
    return result;
}

std::optional<std::string> find_video_device_name(const std::string_view identifier) {
    if (identifier.empty()) return std::nullopt;
    ComPtr<IMFAttributes> attributes;
    check_hresult(MFCreateAttributes(&attributes, 1), "MFCreateAttributes");
    check_hresult(attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID),
                  "Set capture source type");
    IMFActivate** raw = nullptr;
    UINT32 count = 0;
    check_hresult(MFEnumDeviceSources(attributes.Get(), &raw, &count), "MFEnumDeviceSources");
    std::optional<std::string> result;
    for (UINT32 i = 0; i < count; ++i) {
        ComPtr<IMFActivate> activate;
        activate.Attach(raw[i]);
        CoTaskMemString id;
        UINT32 id_length = 0;
        activate->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &id.value, &id_length);
        if (!id.value || utf8(id.value) != identifier) continue;
        CoTaskMemString name;
        UINT32 name_length = 0;
        activate->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name.value, &name_length);
        if (name.value) result = utf8(name.value);
    }
    CoTaskMemFree(raw);
    return result;
}

std::vector<MonitorDevice> enumerate_monitors() {
    std::vector<MonitorDevice> result;
    EnumDisplayMonitors(nullptr, nullptr, monitor_callback, reinterpret_cast<LPARAM>(&result));
    return result;
}

} // namespace asc::win
