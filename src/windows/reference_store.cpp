#include "reference_store.hpp"

#include <vector>

namespace asc::win {
namespace {
GrayImage decode_thumbnail(IWICImagingFactory* factory, const std::filesystem::path& path, const Size target) {
    ComPtr<IWICBitmapDecoder> decoder;
    check_hresult(factory->CreateDecoderFromFilename(path.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad, &decoder), "Decode reference image");
    ComPtr<IWICBitmapFrameDecode> frame;
    check_hresult(decoder->GetFrame(0, &frame), "Read reference frame");
    ComPtr<IWICBitmapScaler> scaler;
    check_hresult(factory->CreateBitmapScaler(&scaler), "Create reference scaler");
    check_hresult(scaler->Initialize(frame.Get(), target.width, target.height, WICBitmapInterpolationModeFant), "Scale reference image");
    ComPtr<IWICFormatConverter> converter;
    check_hresult(factory->CreateFormatConverter(&converter), "Create reference converter");
    check_hresult(converter->Initialize(scaler.Get(), GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, nullptr, 0,
                                        WICBitmapPaletteTypeCustom), "Convert reference image");
    const auto stride = target.width * 4;
    std::vector<std::uint8_t> pixels(static_cast<std::size_t>(stride) * target.height);
    check_hresult(converter->CopyPixels(nullptr, stride, static_cast<UINT>(pixels.size()), pixels.data()), "Copy reference pixels");
    return bgra_to_gray(pixels, target, stride);
}

void encode_png(IWICImagingFactory* factory, const std::filesystem::path& path, const std::uint8_t* pixels,
                const Size size, const std::uint32_t stride) {
    ComPtr<IWICStream> stream;
    check_hresult(factory->CreateStream(&stream), "Create reference file stream");
    check_hresult(stream->InitializeFromFilename(path.c_str(), GENERIC_WRITE), "Open reference image output");
    ComPtr<IWICBitmapEncoder> encoder;
    check_hresult(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder), "Create PNG encoder");
    check_hresult(encoder->Initialize(stream.Get(), WICBitmapEncoderNoCache), "Initialize PNG encoder");
    ComPtr<IWICBitmapFrameEncode> frame;
    check_hresult(encoder->CreateNewFrame(&frame, nullptr), "Create PNG frame");
    check_hresult(frame->Initialize(nullptr), "Initialize PNG frame");
    check_hresult(frame->SetSize(size.width, size.height), "Set PNG dimensions");
    WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
    check_hresult(frame->SetPixelFormat(&format), "Set PNG format");
    check_hresult(frame->WritePixels(size.height, stride, stride * size.height, const_cast<BYTE*>(pixels)), "Write reference PNG");
    check_hresult(frame->Commit(), "Commit reference PNG frame");
    check_hresult(encoder->Commit(), "Commit reference PNG");
}
}

ReferenceStore::ReferenceStore(D3DDevice& d3d) : d3d_(d3d) {
    check_hresult(CoCreateInstance(CLSID_WICImagingFactory2, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&factory_)), "Create WIC factory");
}
GrayImage ReferenceStore::load_thumbnail(const std::filesystem::path& path, const Size size) { return decode_thumbnail(factory_.Get(), path, size); }

GrayImage ReferenceStore::save_frame(const VideoFrame& frame, const std::filesystem::path& path, const Size thumbnail_size) {
    if (!frame.valid()) throw std::runtime_error("no valid screen frame to save as reference");
    D3D11_TEXTURE2D_DESC desc{}; frame.texture->GetDesc(&desc);
    desc.BindFlags = 0; desc.MiscFlags = 0; desc.Usage = D3D11_USAGE_STAGING; desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ComPtr<ID3D11Texture2D> staging;
    check_hresult(d3d_.device()->CreateTexture2D(&desc, nullptr, &staging), "Create reference readback texture");
    d3d_.context()->CopyResource(staging.Get(), frame.texture.Get());
    D3D11_MAPPED_SUBRESOURCE mapped{};
    check_hresult(d3d_.context()->Map(staging.Get(), 0, D3D11_MAP_READ, 0, &mapped), "Map reference screen frame");
    try {
        std::filesystem::create_directories(path.parent_path());
        encode_png(factory_.Get(), path, static_cast<const std::uint8_t*>(mapped.pData), frame.size, mapped.RowPitch);
        const auto bytes = std::span{static_cast<const std::uint8_t*>(mapped.pData), static_cast<std::size_t>(mapped.RowPitch) * frame.size.height};
        auto thumbnail = resize_bilinear(bgra_to_gray(bytes, frame.size, mapped.RowPitch), thumbnail_size);
        d3d_.context()->Unmap(staging.Get(), 0);
        return thumbnail;
    } catch (...) { d3d_.context()->Unmap(staging.Get(), 0); throw; }
}

GrayImage ReferenceStore::import_image(const std::filesystem::path& source, const std::filesystem::path& destination,
                                       const Size thumbnail_size) {
    ComPtr<IWICBitmapDecoder> decoder;
    check_hresult(factory_->CreateDecoderFromFilename(source.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad, &decoder), "Decode imported reference");
    ComPtr<IWICBitmapFrameDecode> frame;
    check_hresult(decoder->GetFrame(0, &frame), "Read imported reference");
    UINT width = 0, height = 0; frame->GetSize(&width, &height);
    if (width == 0 || height == 0 || width > 16384 || height > 16384 || static_cast<std::uint64_t>(width) * height > 100'000'000)
        throw std::runtime_error("imported reference image dimensions are not supported");
    ComPtr<IWICFormatConverter> converter;
    check_hresult(factory_->CreateFormatConverter(&converter), "Create imported reference converter");
    check_hresult(converter->Initialize(frame.Get(), GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, nullptr, 0,
                                        WICBitmapPaletteTypeCustom), "Convert imported reference");
    const auto stride = width * 4;
    std::vector<std::uint8_t> pixels(static_cast<std::size_t>(stride) * height);
    check_hresult(converter->CopyPixels(nullptr, stride, static_cast<UINT>(pixels.size()), pixels.data()), "Copy imported reference");
    std::filesystem::create_directories(destination.parent_path());
    encode_png(factory_.Get(), destination, pixels.data(), {width, height}, stride);
    return resize_bilinear(bgra_to_gray(pixels, {width, height}, stride), thumbnail_size);
}

} // namespace asc::win
