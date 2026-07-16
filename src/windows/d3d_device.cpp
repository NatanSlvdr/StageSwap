#include "d3d_device.hpp"

#include <d3d10.h>

namespace asc::win {

D3DDevice::D3DDevice() { create(); }

void D3DDevice::create() {
    device_.Reset();
    context_.Reset();
    constexpr D3D_FEATURE_LEVEL levels[]{D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0};
    UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
#ifdef _DEBUG
    flags |= D3D11_CREATE_DEVICE_DEBUG;
#endif
    D3D_FEATURE_LEVEL selected{};
    auto result = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, flags, levels, ARRAYSIZE(levels),
                                    D3D11_SDK_VERSION, &device_, &selected, &context_);
#ifdef _DEBUG
    if (result == DXGI_ERROR_SDK_COMPONENT_MISSING) {
        flags &= ~D3D11_CREATE_DEVICE_DEBUG;
        result = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, flags, levels, ARRAYSIZE(levels),
                                   D3D11_SDK_VERSION, &device_, &selected, &context_);
    }
#endif
    check_hresult(result, "D3D11CreateDevice");
    ComPtr<ID3D10Multithread> multithread;
    check_hresult(device_.As(&multithread), "Query ID3D10Multithread");
    multithread->SetMultithreadProtected(TRUE);
}

} // namespace asc::win
