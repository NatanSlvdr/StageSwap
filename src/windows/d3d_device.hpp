#pragma once

#include "common.hpp"

#include <d3d11_4.h>
#include <dxgi1_6.h>

namespace asc::win {

class D3DDevice {
public:
    D3DDevice();
    [[nodiscard]] ID3D11Device* device() const noexcept { return device_.Get(); }
    [[nodiscard]] ID3D11DeviceContext* context() const noexcept { return context_.Get(); }

private:
    void create();
    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11DeviceContext> context_;
};

} // namespace asc::win
