#pragma once

#include "common.hpp"
#include <mfidl.h>
#include <wrl/implements.h>
#include <mutex>

namespace asc::win::source {

class Activation final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IMFActivate> {
public:
    Activation();
    STDMETHODIMP ActivateObject(REFIID riid, void** object) override;
    STDMETHODIMP ShutdownObject() override;
    STDMETHODIMP DetachObject() override;

    STDMETHODIMP GetItem(REFGUID key, PROPVARIANT* value) override;
    STDMETHODIMP GetItemType(REFGUID key, MF_ATTRIBUTE_TYPE* type) override;
    STDMETHODIMP CompareItem(REFGUID key, REFPROPVARIANT value, BOOL* result) override;
    STDMETHODIMP Compare(IMFAttributes* theirs, MF_ATTRIBUTES_MATCH_TYPE type, BOOL* result) override;
    STDMETHODIMP GetUINT32(REFGUID key, UINT32* value) override;
    STDMETHODIMP GetUINT64(REFGUID key, UINT64* value) override;
    STDMETHODIMP GetDouble(REFGUID key, double* value) override;
    STDMETHODIMP GetGUID(REFGUID key, GUID* value) override;
    STDMETHODIMP GetStringLength(REFGUID key, UINT32* length) override;
    STDMETHODIMP GetString(REFGUID key, LPWSTR value, UINT32 size, UINT32* length) override;
    STDMETHODIMP GetAllocatedString(REFGUID key, LPWSTR* value, UINT32* length) override;
    STDMETHODIMP GetBlobSize(REFGUID key, UINT32* size) override;
    STDMETHODIMP GetBlob(REFGUID key, UINT8* buffer, UINT32 size, UINT32* blob_size) override;
    STDMETHODIMP GetAllocatedBlob(REFGUID key, UINT8** buffer, UINT32* size) override;
    STDMETHODIMP GetUnknown(REFGUID key, REFIID riid, void** object) override;
    STDMETHODIMP SetItem(REFGUID key, REFPROPVARIANT value) override;
    STDMETHODIMP DeleteItem(REFGUID key) override;
    STDMETHODIMP DeleteAllItems() override;
    STDMETHODIMP SetUINT32(REFGUID key, UINT32 value) override;
    STDMETHODIMP SetUINT64(REFGUID key, UINT64 value) override;
    STDMETHODIMP SetDouble(REFGUID key, double value) override;
    STDMETHODIMP SetGUID(REFGUID key, REFGUID value) override;
    STDMETHODIMP SetString(REFGUID key, LPCWSTR value) override;
    STDMETHODIMP SetBlob(REFGUID key, const UINT8* buffer, UINT32 size) override;
    STDMETHODIMP SetUnknown(REFGUID key, IUnknown* value) override;
    STDMETHODIMP LockStore() override;
    STDMETHODIMP UnlockStore() override;
    STDMETHODIMP GetCount(UINT32* count) override;
    STDMETHODIMP GetItemByIndex(UINT32 index, GUID* key, PROPVARIANT* value) override;
    STDMETHODIMP CopyAllItems(IMFAttributes* destination) override;

private:
    std::mutex mutex_;
    HRESULT initialization_result_{E_UNEXPECTED};
    ComPtr<IMFAttributes> attributes_;
    ComPtr<IMFMediaSource> active_source_;
};

} // namespace asc::win::source
