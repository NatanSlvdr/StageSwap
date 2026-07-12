#include "activation.hpp"
#include "ids.hpp"

#include <windows.h>
#include <mfapi.h>
#include <wrl/implements.h>
#include <wrl/module.h>
#include <string>

using namespace asc::win;
using namespace asc::win::source;

namespace {
HMODULE module_handle = nullptr;

class ClassFactory final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IClassFactory> {
public:
    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID riid, void** object) override {
        if (outer) return CLASS_E_NOAGGREGATION;
        if (!object) return E_POINTER;
        auto activation = Microsoft::WRL::Make<Activation>();
        return activation ? activation->QueryInterface(riid, object) : E_OUTOFMEMORY;
    }
    STDMETHODIMP LockServer(BOOL) override { return S_OK; }
};

HRESULT register_server(const bool install) {
    wchar_t module_path[MAX_PATH]{};
    if (!GetModuleFileNameW(module_handle, module_path, ARRAYSIZE(module_path))) return HRESULT_FROM_WIN32(GetLastError());
    wchar_t clsid[64]{};
    StringFromGUID2(CLSID_AutomaticScreenCameraSource, clsid, ARRAYSIZE(clsid));
    const std::wstring key_path = std::wstring(L"Software\\Classes\\CLSID\\") + clsid;
    if (!install) {
        const auto status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, key_path.c_str());
        return status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND ? S_OK : HRESULT_FROM_WIN32(status);
    }
    HKEY class_key = nullptr;
    auto status = RegCreateKeyExW(HKEY_LOCAL_MACHINE, key_path.c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &class_key, nullptr);
    if (status != ERROR_SUCCESS) return HRESULT_FROM_WIN32(status);
    const wchar_t friendly[] = L"Automatic Screen Camera Media Source";
    RegSetValueExW(class_key, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(friendly), sizeof(friendly));
    HKEY server_key = nullptr;
    status = RegCreateKeyExW(class_key, L"InprocServer32", 0, nullptr, 0, KEY_WRITE, nullptr, &server_key, nullptr);
    if (status == ERROR_SUCCESS) {
        RegSetValueExW(server_key, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(module_path),
                       static_cast<DWORD>((wcslen(module_path) + 1) * sizeof(wchar_t)));
        const wchar_t threading[] = L"Both";
        RegSetValueExW(server_key, L"ThreadingModel", 0, REG_SZ, reinterpret_cast<const BYTE*>(threading), sizeof(threading));
        RegCloseKey(server_key);
    }
    RegCloseKey(class_key);
    return HRESULT_FROM_WIN32(status);
}
}

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) { module_handle = instance; DisableThreadLibraryCalls(instance); }
    return TRUE;
}

extern "C" __declspec(dllexport) HRESULT WINAPI DllGetClassObject(REFCLSID clsid, REFIID riid, void** object) {
    if (clsid != CLSID_AutomaticScreenCameraSource) return CLASS_E_CLASSNOTAVAILABLE;
    auto factory = Microsoft::WRL::Make<ClassFactory>();
    return factory ? factory->QueryInterface(riid, object) : E_OUTOFMEMORY;
}
extern "C" __declspec(dllexport) HRESULT WINAPI DllCanUnloadNow() {
    return Microsoft::WRL::Module<Microsoft::WRL::InProc>::GetModule().GetObjectCount() == 0 ? S_OK : S_FALSE;
}
extern "C" __declspec(dllexport) HRESULT WINAPI DllRegisterServer() { return register_server(true); }
extern "C" __declspec(dllexport) HRESULT WINAPI DllUnregisterServer() { return register_server(false); }
