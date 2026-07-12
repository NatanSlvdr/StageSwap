#pragma once

#include "common.hpp"

#include <d3d11_4.h>
#include <dxgi1_6.h>
#include <mfidl.h>

namespace asc::win {

class D3DDevice {
public:
    D3DDevice();
    [[nodiscard]] ID3D11Device* device() const noexcept { return device_.Get(); }
    [[nodiscard]] ID3D11DeviceContext* context() const noexcept { return context_.Get(); }
    [[nodiscard]] IMFDXGIDeviceManager* mf_manager() const noexcept { return mf_manager_.Get(); }
    [[nodiscard]] UINT mf_reset_token() const noexcept { return mf_reset_token_; }
    void reset_after_device_loss();

private:
    void create();
    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<IMFDXGIDeviceManager> mf_manager_;
    UINT mf_reset_token_{0};
};

} // namespace asc::win

