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
BOOL CALLBACK monitor_callback(HMONITOR handle, HDC, RECT*, LPARAM context) {
    auto& output = *reinterpret_cast<std::vector<MonitorDevice>*>(context);
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    if (!GetMonitorInfoW(handle, &info)) return TRUE;
    DISPLAY_DEVICEW adapter{};
    adapter.cb = sizeof(adapter);
    EnumDisplayDevicesW(info.szDevice, 0, &adapter, EDD_GET_DEVICE_INTERFACE_NAME);
    asc::RuntimeMonitorDescriptor descriptor;
    descriptor.gdi_display_name = utf8(info.szDevice);
    descriptor.label = utf8(adapter.DeviceString[0] ? adapter.DeviceString : info.szDevice);
    descriptor.geometry = {info.rcMonitor.left, info.rcMonitor.top,
        static_cast<std::uint32_t>(info.rcMonitor.right - info.rcMonitor.left),
        static_cast<std::uint32_t>(info.rcMonitor.bottom - info.rcMonitor.top)};
    descriptor.native_handle = reinterpret_cast<std::uintptr_t>(handle);
    output.push_back({handle, std::move(descriptor)});
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
        VideoDevice device{name.value ? utf8(name.value) : "Unnamed video device", id.value ? utf8(id.value) : "", true};
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
