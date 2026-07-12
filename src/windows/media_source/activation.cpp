#include "activation.hpp"
#include "media_source.hpp"

#include <mfapi.h>

namespace asc::win::source {

Activation::Activation() { MFCreateAttributes(&attributes_, 4); }

HRESULT Activation::ActivateObject(const REFIID riid, void** object) {
    if (!object) return E_POINTER;
    *object = nullptr;
    auto source = Microsoft::WRL::Make<MediaSource>();
    if (!source) return E_OUTOFMEMORY;
    auto result = source->initialize(attributes_.Get());
    if (FAILED(result)) return result;
    result = source.As(&active_source_);
    if (FAILED(result)) return result;
    return active_source_->QueryInterface(riid, object);
}
HRESULT Activation::ShutdownObject() { if (active_source_) active_source_->Shutdown(); return S_OK; }
HRESULT Activation::DetachObject() { if (active_source_) active_source_->Shutdown(); active_source_.Reset(); return S_OK; }

#define ASC_ATTR_DELEGATE(method, signature, args) HRESULT Activation::method signature { return attributes_->method args; }
ASC_ATTR_DELEGATE(GetItem, (REFGUID k, PROPVARIANT* v), (k, v))
ASC_ATTR_DELEGATE(GetItemType, (REFGUID k, MF_ATTRIBUTE_TYPE* v), (k, v))
ASC_ATTR_DELEGATE(CompareItem, (REFGUID k, REFPROPVARIANT v, BOOL* r), (k, v, r))
ASC_ATTR_DELEGATE(Compare, (IMFAttributes* v, MF_ATTRIBUTES_MATCH_TYPE t, BOOL* r), (v, t, r))
ASC_ATTR_DELEGATE(GetUINT32, (REFGUID k, UINT32* v), (k, v))
ASC_ATTR_DELEGATE(GetUINT64, (REFGUID k, UINT64* v), (k, v))
ASC_ATTR_DELEGATE(GetDouble, (REFGUID k, double* v), (k, v))
ASC_ATTR_DELEGATE(GetGUID, (REFGUID k, GUID* v), (k, v))
ASC_ATTR_DELEGATE(GetStringLength, (REFGUID k, UINT32* v), (k, v))
ASC_ATTR_DELEGATE(GetString, (REFGUID k, LPWSTR v, UINT32 s, UINT32* l), (k, v, s, l))
ASC_ATTR_DELEGATE(GetAllocatedString, (REFGUID k, LPWSTR* v, UINT32* l), (k, v, l))
ASC_ATTR_DELEGATE(GetBlobSize, (REFGUID k, UINT32* v), (k, v))
ASC_ATTR_DELEGATE(GetBlob, (REFGUID k, UINT8* v, UINT32 s, UINT32* l), (k, v, s, l))
ASC_ATTR_DELEGATE(GetAllocatedBlob, (REFGUID k, UINT8** v, UINT32* s), (k, v, s))
ASC_ATTR_DELEGATE(GetUnknown, (REFGUID k, REFIID i, void** v), (k, i, v))
ASC_ATTR_DELEGATE(SetItem, (REFGUID k, REFPROPVARIANT v), (k, v))
ASC_ATTR_DELEGATE(DeleteItem, (REFGUID k), (k))
ASC_ATTR_DELEGATE(DeleteAllItems, (), ())
ASC_ATTR_DELEGATE(SetUINT32, (REFGUID k, UINT32 v), (k, v))
ASC_ATTR_DELEGATE(SetUINT64, (REFGUID k, UINT64 v), (k, v))
ASC_ATTR_DELEGATE(SetDouble, (REFGUID k, double v), (k, v))
ASC_ATTR_DELEGATE(SetGUID, (REFGUID k, REFGUID v), (k, v))
ASC_ATTR_DELEGATE(SetString, (REFGUID k, LPCWSTR v), (k, v))
ASC_ATTR_DELEGATE(SetBlob, (REFGUID k, const UINT8* v, UINT32 s), (k, v, s))
ASC_ATTR_DELEGATE(SetUnknown, (REFGUID k, IUnknown* v), (k, v))
ASC_ATTR_DELEGATE(LockStore, (), ())
ASC_ATTR_DELEGATE(UnlockStore, (), ())
ASC_ATTR_DELEGATE(GetCount, (UINT32* v), (v))
ASC_ATTR_DELEGATE(GetItemByIndex, (UINT32 i, GUID* k, PROPVARIANT* v), (i, k, v))
ASC_ATTR_DELEGATE(CopyAllItems, (IMFAttributes* v), (v))
#undef ASC_ATTR_DELEGATE

} // namespace asc::win::source
