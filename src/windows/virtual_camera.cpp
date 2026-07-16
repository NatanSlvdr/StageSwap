#include "virtual_camera.hpp"

#include <mfapi.h>
#include <wrl/implements.h>
#include <new>
#include <utility>

namespace asc::win {
namespace {
class CameraEventCallback final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IMFAsyncCallback> {
public:
    CameraEventCallback(std::shared_ptr<VirtualCameraCallbackState> state, const std::uint64_t expected)
        : state_(std::move(state)), expected_(expected) {}
    STDMETHODIMP GetParameters(DWORD*, DWORD*) override { return E_NOTIMPL; }
    STDMETHODIMP Invoke(IMFAsyncResult* result) override {
        if (state_->generation != expected_) return S_OK;
        if (!result || FAILED(result->GetStatus())) { state_->running = false; return S_OK; }
        ComPtr<IUnknown> object;
        ComPtr<IMFMediaEvent> event;
        if (SUCCEEDED(result->GetObject(&object)) && object && SUCCEEDED(object.As(&event))) {
            MediaEventType type{};
            HRESULT status = S_OK;
            event->GetType(&type); event->GetStatus(&status);
            if (type == MEError || FAILED(status)) state_->running = false;
        }
        return S_OK;
    }
private:
    std::shared_ptr<VirtualCameraCallbackState> state_;
    std::uint64_t expected_;
};
}

VirtualCamera::~VirtualCamera() { stop(); }

void VirtualCamera::start(const std::wstring& pipe_name, const std::uint32_t placeholder_color) {
    std::scoped_lock lock(mutex_);
    stop_unlocked();
    start_unlocked(pipe_name, placeholder_color);
}

void VirtualCamera::start_unlocked(const std::wstring& pipe_name, const std::uint32_t placeholder_color) {
    pipe_name_ = pipe_name;
    placeholder_color_ = placeholder_color | 0xff000000u;
    ComPtr<IMFActivate> registration_check;
    check_hresult(CoCreateInstance(CLSID_AutomaticScreenCameraSource, nullptr, CLSCTX_INPROC_SERVER,
                                   IID_PPV_ARGS(&registration_check)),
                  "Load AutomaticScreenCameraSource (relaunch the portable executable and approve registration if missing)");
    check_hresult(MFCreateVirtualCamera(MFVirtualCameraType_SoftwareCameraSource, MFVirtualCameraLifetime_System,
                                        MFVirtualCameraAccess_CurrentUser, L"Automatic Screen Camera",
                                        CLSID_AutomaticScreenCameraSourceText, nullptr, 0, &camera_),
                  "MFCreateVirtualCamera");
    check_hresult(camera_->SetString(ASC_FRAME_PIPE_NAME, pipe_name_.c_str()), "Configure virtual camera frame pipe");
    check_hresult(camera_->SetUINT32(ASC_PLACEHOLDER_COLOR, placeholder_color_), "Configure virtual camera placeholder");
    const auto generation = ++callback_state_->generation;
    callback_ = Microsoft::WRL::Make<CameraEventCallback>(callback_state_, generation);
    if (!callback_) throw std::bad_alloc();
    callback_state_->running = true;
    const auto start_result = camera_->Start(callback_.Get());
    if (FAILED(start_result)) {
        callback_state_->running = false;
        check_hresult(start_result, "IMFVirtualCamera::Start");
    }
    CoTaskMemString link;
    UINT32 length = 0;
    if (SUCCEEDED(camera_->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &link.value, &length)) && link.value)
        symbolic_link_ = link.value;
}

void VirtualCamera::stop() noexcept {
    std::scoped_lock lock(mutex_);
    stop_unlocked();
}

void VirtualCamera::stop_unlocked() noexcept {
    ++callback_state_->generation;
    // This is a system-lifetime camera. IMFVirtualCamera::Stop invalidates every
    // active consumer, which would prevent the out-of-process media source from
    // switching to its privacy placeholder when the tray exits. Releasing this
    // controller leaves registration and existing Frame Server sessions alive;
    // Portable cleanup is the only path that calls Remove().
    camera_.Reset();
    callback_.Reset();
    callback_state_->running = false;
}

void VirtualCamera::restart() {
    std::scoped_lock lock(mutex_);
    const auto name = pipe_name_;
    stop_unlocked();
    start_unlocked(name, placeholder_color_);
}

void VirtualCamera::remove_registration() {
    ComPtr<IMFVirtualCamera> camera;
    check_hresult(MFCreateVirtualCamera(MFVirtualCameraType_SoftwareCameraSource, MFVirtualCameraLifetime_System,
                                        MFVirtualCameraAccess_CurrentUser, L"Automatic Screen Camera",
                                        CLSID_AutomaticScreenCameraSourceText, nullptr, 0, &camera),
                  "Open virtual camera registration");
    check_hresult(camera->Remove(), "Remove virtual camera registration");
    camera->Shutdown();
}

} // namespace asc::win
