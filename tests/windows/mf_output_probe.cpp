#include <windows.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <psapi.h>
#include <wrl/client.h>
#include <wrl/implements.h>

#include <algorithm>
#include <chrono>
#include <climits>
#include <cmath>
#include <condition_variable>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <iterator>
#include <mutex>
#include <optional>
#include <numeric>
#include <sstream>
#include <string>
#include <string_view>
#include <thread>
#include <tlhelp32.h>
#include <utility>
#include <vector>

using Microsoft::WRL::ComPtr;

namespace {

constexpr DWORD first_video_stream = static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM);

struct Options {
    std::wstring camera_name{L"Automatic Screen Camera"};
    std::filesystem::path output{L"mf-probe-results.json"};
    unsigned duration_seconds{60};
    unsigned warmup_seconds{60};
    unsigned hash_sample_every{30};
    double minimum_fps{29.0};
    double maximum_fps{31.0};
    double maximum_stale_ms{2000.0};
};

struct ProcessSample {
    std::chrono::steady_clock::time_point wall;
    std::uint64_t cpu_100ns{};
    std::uint64_t private_bytes{};
    DWORD handles{};
};

struct HashSample {
    std::uint64_t frame{};
    LONGLONG timestamp{};
    LONGLONG qpc_ticks{};
    std::uint64_t hash{};
};

struct AsyncReadResult {
    HRESULT status{E_UNEXPECTED};
    DWORD flags{};
    LONGLONG timestamp{};
    ComPtr<IMFSample> sample;
};

class ProbeReaderCallback final : public Microsoft::WRL::RuntimeClass<
    Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IMFSourceReaderCallback> {
public:
    STDMETHODIMP OnReadSample(const HRESULT status, DWORD, const DWORD flags,
                              const LONGLONG timestamp, IMFSample* sample) override {
        {
            std::scoped_lock lock(mutex_);
            AsyncReadResult result{status, flags, timestamp, {}};
            result.sample = sample;
            result_ = std::move(result);
        }
        condition_.notify_all();
        return S_OK;
    }

    STDMETHODIMP OnFlush(DWORD) override {
        {
            std::scoped_lock lock(mutex_);
            if (!result_) result_.emplace(AsyncReadResult{MF_E_SHUTDOWN, MF_SOURCE_READERF_ERROR, 0, {}});
        }
        condition_.notify_all();
        return S_OK;
    }

    STDMETHODIMP OnEvent(DWORD, IMFMediaEvent*) override { return S_OK; }

    bool wait_until(const std::chrono::steady_clock::time_point deadline, AsyncReadResult& output) {
        std::unique_lock lock(mutex_);
        if (!condition_.wait_until(lock, deadline, [this] { return result_.has_value(); })) return false;
        output = std::move(*result_);
        result_.reset();
        return true;
    }

private:
    std::mutex mutex_;
    std::condition_variable condition_;
    std::optional<AsyncReadResult> result_;
};

std::optional<unsigned> parse_unsigned(const std::wstring_view value) {
    if (value.empty()) return std::nullopt;
    unsigned result{};
    for (const wchar_t character : value) {
        if (character < L'0' || character > L'9') return std::nullopt;
        const unsigned digit = static_cast<unsigned>(character - L'0');
        if (result > (UINT_MAX - digit) / 10U) return std::nullopt;
        result = result * 10U + digit;
    }
    return result;
}

std::optional<double> parse_double(const std::wstring_view value) {
    try {
        std::size_t consumed{};
        const double result = std::stod(std::wstring{value}, &consumed);
        if (consumed != value.size() || !std::isfinite(result)) return std::nullopt;
        return result;
    } catch (...) {
        return std::nullopt;
    }
}

bool parse_options(const int argc, wchar_t** argv, Options& result) {
    for (int index = 1; index < argc; ++index) {
        const std::wstring_view argument{argv[index]};
        const auto require_value = [&](std::wstring_view& value) {
            if (index + 1 >= argc) return false;
            value = argv[++index];
            return true;
        };
        std::wstring_view value;
        if (argument == L"--camera-name") {
            if (!require_value(value) || value.empty()) return false;
            result.camera_name = value;
        } else if (argument == L"--output") {
            if (!require_value(value) || value.empty()) return false;
            result.output = value;
        } else if (argument == L"--duration-seconds") {
            if (!require_value(value)) return false;
            const auto parsed = parse_unsigned(value);
            if (!parsed || *parsed == 0 || *parsed > 172'800) return false;
            result.duration_seconds = *parsed;
        } else if (argument == L"--hash-sample-every") {
            if (!require_value(value)) return false;
            const auto parsed = parse_unsigned(value);
            if (!parsed || *parsed == 0 || *parsed > 3600) return false;
            result.hash_sample_every = *parsed;
        } else if (argument == L"--warmup-seconds") {
            if (!require_value(value)) return false;
            const auto parsed = parse_unsigned(value);
            if (!parsed || *parsed > 3600) return false;
            result.warmup_seconds = *parsed;
        } else if (argument == L"--minimum-fps") {
            if (!require_value(value)) return false;
            const auto parsed = parse_double(value);
            if (!parsed || *parsed <= 0.0) return false;
            result.minimum_fps = *parsed;
        } else if (argument == L"--maximum-fps") {
            if (!require_value(value)) return false;
            const auto parsed = parse_double(value);
            if (!parsed || *parsed <= 0.0) return false;
            result.maximum_fps = *parsed;
        } else if (argument == L"--maximum-stale-ms") {
            if (!require_value(value)) return false;
            const auto parsed = parse_double(value);
            if (!parsed || *parsed < 0.0) return false;
            result.maximum_stale_ms = *parsed;
        } else if (argument == L"--help" || argument == L"-h") {
            std::wcout << L"Usage: asc_mf_output_probe [--camera-name NAME] [--output FILE] "
                          L"[--duration-seconds N] [--hash-sample-every N] "
                          L"[--warmup-seconds N] [--minimum-fps N] [--maximum-fps N] "
                          L"[--maximum-stale-ms N]\n";
            std::exit(0);
        } else {
            return false;
        }
    }
    return result.minimum_fps <= result.maximum_fps;
}

std::string utf8(const std::wstring_view value) {
    if (value.empty()) return {};
    const int count = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
                                          static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    if (count <= 0) return {};
    std::string result(static_cast<std::size_t>(count), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()),
                            result.data(), count, nullptr, nullptr) != count) return {};
    return result;
}

std::string json_escape(const std::string_view value) {
    std::ostringstream result;
    for (const unsigned char character : value) {
        switch (character) {
        case '\\': result << "\\\\"; break;
        case '"': result << "\\\""; break;
        case '\b': result << "\\b"; break;
        case '\f': result << "\\f"; break;
        case '\n': result << "\\n"; break;
        case '\r': result << "\\r"; break;
        case '\t': result << "\\t"; break;
        default:
            if (character < 0x20) result << "\\u" << std::hex << std::setw(4) << std::setfill('0') << static_cast<unsigned>(character);
            else result << static_cast<char>(character);
        }
    }
    return result.str();
}

std::string guid_string(const GUID& guid) {
    wchar_t value[39]{};
    if (StringFromGUID2(guid, value, static_cast<int>(std::size(value))) == 0) return {};
    return utf8(value);
}

const char* subtype_name(const GUID& subtype) {
    if (IsEqualGUID(subtype, MFVideoFormat_RGB32)) return "RGB32";
    if (IsEqualGUID(subtype, MFVideoFormat_NV12)) return "NV12";
    return "unknown";
}

std::uint64_t file_time_value(const FILETIME& value) {
    ULARGE_INTEGER integer{};
    integer.LowPart = value.dwLowDateTime;
    integer.HighPart = value.dwHighDateTime;
    return integer.QuadPart;
}

std::optional<ProcessSample> sample_process(HANDLE process) {
    FILETIME creation{}, exit{}, kernel{}, user{};
    PROCESS_MEMORY_COUNTERS_EX memory{};
    memory.cb = static_cast<DWORD>(sizeof(memory));
    DWORD handles{};
    if (!GetProcessTimes(process, &creation, &exit, &kernel, &user) ||
        !GetProcessMemoryInfo(process, reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&memory), static_cast<DWORD>(sizeof(memory))) ||
        !GetProcessHandleCount(process, &handles)) return std::nullopt;
    return ProcessSample{
        std::chrono::steady_clock::now(),
        file_time_value(kernel) + file_time_value(user),
        static_cast<std::uint64_t>(memory.PrivateUsage),
        handles,
    };
}

HANDLE open_process_by_name(const wchar_t* executable_name) {
    const HANDLE snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (snapshot == INVALID_HANDLE_VALUE) return nullptr;
    PROCESSENTRY32W entry{static_cast<DWORD>(sizeof(entry))};
    HANDLE result{};
    if (Process32FirstW(snapshot, &entry)) {
        do {
            if (_wcsicmp(entry.szExeFile, executable_name) == 0) {
                result = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, FALSE, entry.th32ProcessID);
                break;
            }
        } while (Process32NextW(snapshot, &entry));
    }
    CloseHandle(snapshot);
    return result;
}

ComPtr<IMFMediaSource> open_camera(const std::wstring_view requested_name, std::wstring& actual_name) {
    ComPtr<IMFAttributes> attributes;
    if (FAILED(MFCreateAttributes(&attributes, 1)) ||
        FAILED(attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                                   MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID))) return {};

    IMFActivate** devices{};
    UINT32 count{};
    if (FAILED(MFEnumDeviceSources(attributes.Get(), &devices, &count))) return {};
    ComPtr<IMFMediaSource> result;
    for (UINT32 index = 0; index < count; ++index) {
        wchar_t* name{};
        UINT32 name_length{};
        if (SUCCEEDED(devices[index]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name, &name_length))) {
            const std::wstring_view candidate{name, name_length};
            if (candidate == requested_name ||
                (candidate.starts_with(requested_name) && candidate.size() > requested_name.size() &&
                 (candidate[requested_name.size()] == L' ' || candidate[requested_name.size()] == L'('))) {
                actual_name.assign(candidate);
                devices[index]->ActivateObject(IID_PPV_ARGS(&result));
            }
            CoTaskMemFree(name);
        }
        devices[index]->Release();
    }
    CoTaskMemFree(devices);
    return result;
}

std::uint64_t hash_bytes(const BYTE* data, const DWORD size) {
    std::uint64_t hash = 1469598103934665603ULL;
    for (DWORD index = 0; index < size; ++index) {
        hash ^= data[index];
        hash *= 1099511628211ULL;
    }
    return hash;
}

double percentile(std::vector<double> values, const double fraction) {
    if (values.empty()) return 0.0;
    std::sort(values.begin(), values.end());
    const auto index = static_cast<std::size_t>(std::ceil(fraction * static_cast<double>(values.size()))) - 1U;
    return values[std::min(index, values.size() - 1U)];
}

} // namespace

int wmain(const int argc, wchar_t** argv) {
    Options options;
    if (!parse_options(argc, argv, options)) {
        std::wcerr << L"Invalid arguments. Run with --help for usage.\n";
        return 2;
    }

    const HRESULT com_result = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(com_result)) {
        std::wcerr << L"COM initialization failed.\n";
        return 3;
    }
    const HRESULT mf_result = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    if (FAILED(mf_result)) {
        CoUninitialize();
        std::wcerr << L"Media Foundation initialization failed.\n";
        return 3;
    }

    int exit_code = 4;
    std::wstring actual_name;
    ComPtr<IMFMediaSource> source = open_camera(options.camera_name, actual_name);
    if (!source) {
        std::wcerr << L"The requested camera was not found or could not be activated: " << options.camera_name << L'\n';
    } else {
        ComPtr<IMFAttributes> reader_attributes;
        ComPtr<IMFSourceReader> reader;
        const auto reader_callback = Microsoft::WRL::Make<ProbeReaderCallback>();
        if (reader_callback && SUCCEEDED(MFCreateAttributes(&reader_attributes, 2)) &&
            SUCCEEDED(reader_attributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, reader_callback.Get())) &&
            SUCCEEDED(reader_attributes->SetUINT32(MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, TRUE)) &&
            SUCCEEDED(MFCreateSourceReaderFromMediaSource(source.Get(), reader_attributes.Get(), &reader))) {
            ComPtr<IMFMediaType> media_type;
            GUID subtype{};
            UINT32 width{}, height{}, numerator{}, denominator{};
            if (SUCCEEDED(reader->GetCurrentMediaType(first_video_stream, &media_type))) {
                media_type->GetGUID(MF_MT_SUBTYPE, &subtype);
                MFGetAttributeSize(media_type.Get(), MF_MT_FRAME_SIZE, &width, &height);
                MFGetAttributeRatio(media_type.Get(), MF_MT_FRAME_RATE, &numerator, &denominator);
            }

            HANDLE producer = open_process_by_name(L"windows-x64-portable.exe");
            if (!producer) producer = open_process_by_name(L"windows-arm64-portable.exe");
            std::vector<ProcessSample> process_samples;
            if (producer != nullptr) {
                if (const auto sample = sample_process(producer)) process_samples.push_back(*sample);
            }

            const auto started = std::chrono::steady_clock::now();
            auto next_process_sample = started + std::chrono::seconds{1};
            const auto deadline = started + std::chrono::seconds{options.duration_seconds};
            LARGE_INTEGER qpc_frequency{};
            LARGE_INTEGER qpc_origin{};
            if (!QueryPerformanceFrequency(&qpc_frequency) || !QueryPerformanceCounter(&qpc_origin)) {
                qpc_frequency.QuadPart = 0;
                qpc_origin.QuadPart = 0;
            }
            std::uint64_t frame_count{};
            std::uint64_t hash_changes{};
            std::uint64_t prior_hash{};
            LONGLONG unchanged_hash_started_qpc{};
            LONGLONG maximum_unchanged_hash_qpc{};
            LONGLONG first_timestamp{};
            LONGLONG prior_timestamp{};
            LONGLONG last_timestamp{};
            LONGLONG maximum_gap{};
            LONGLONG prior_qpc{};
            LONGLONG maximum_delivery_gap_qpc{};
            unsigned timestamp_regressions{};
            unsigned read_failures{};
            std::vector<HashSample> hash_samples;

            while (std::chrono::steady_clock::now() < deadline) {
                const HRESULT request_result = reader->ReadSample(first_video_stream, 0, nullptr,
                                                                  nullptr, nullptr, nullptr);
                if (FAILED(request_result)) {
                    ++read_failures;
                    break;
                }
                AsyncReadResult read;
                const auto read_deadline = std::min(deadline, std::chrono::steady_clock::now() + std::chrono::seconds{2});
                if (!reader_callback->wait_until(read_deadline, read)) {
                    reader->Flush(first_video_stream);
                    if (std::chrono::steady_clock::now() < deadline) ++read_failures;
                    break;
                }
                const DWORD flags = read.flags;
                const LONGLONG timestamp = read.timestamp;
                auto sample = std::move(read.sample);
                if (FAILED(read.status)) {
                    ++read_failures;
                    break;
                }
                if ((flags & MF_SOURCE_READERF_ENDOFSTREAM) != 0) break;
                if (!sample) continue;

                LARGE_INTEGER sample_qpc{};
                if (qpc_frequency.QuadPart != 0) QueryPerformanceCounter(&sample_qpc);

                ComPtr<IMFMediaBuffer> buffer;
                if (FAILED(sample->ConvertToContiguousBuffer(&buffer))) {
                    ++read_failures;
                    continue;
                }
                BYTE* bytes{};
                DWORD maximum_length{}, current_length{};
                if (FAILED(buffer->Lock(&bytes, &maximum_length, &current_length))) {
                    ++read_failures;
                    continue;
                }
                const std::uint64_t hash = hash_bytes(bytes, current_length);
                buffer->Unlock();

                if (frame_count == 0) first_timestamp = timestamp;
                if (frame_count == 0) unchanged_hash_started_qpc = sample_qpc.QuadPart;
                if (frame_count != 0) {
                    if (timestamp <= prior_timestamp) ++timestamp_regressions;
                    else maximum_gap = std::max(maximum_gap, timestamp - prior_timestamp);
                    if (hash != prior_hash) {
                        ++hash_changes;
                        if (sample_qpc.QuadPart > unchanged_hash_started_qpc) {
                            maximum_unchanged_hash_qpc = std::max(maximum_unchanged_hash_qpc,
                                                                  prior_qpc - unchanged_hash_started_qpc);
                        }
                        unchanged_hash_started_qpc = sample_qpc.QuadPart;
                    }
                    if (sample_qpc.QuadPart > prior_qpc) {
                        maximum_delivery_gap_qpc = std::max(maximum_delivery_gap_qpc, sample_qpc.QuadPart - prior_qpc);
                    }
                }
                if ((frame_count % options.hash_sample_every) == 0) {
                    hash_samples.push_back({frame_count, timestamp, sample_qpc.QuadPart, hash});
                }
                prior_hash = hash;
                prior_timestamp = timestamp;
                prior_qpc = sample_qpc.QuadPart;
                last_timestamp = timestamp;
                ++frame_count;

                const auto now = std::chrono::steady_clock::now();
                if (producer != nullptr && now >= next_process_sample) {
                    if (const auto process_sample = sample_process(producer)) process_samples.push_back(*process_sample);
                    next_process_sample = now + std::chrono::seconds{1};
                }
            }
            const auto finished = std::chrono::steady_clock::now();
            if (prior_qpc > unchanged_hash_started_qpc) {
                maximum_unchanged_hash_qpc = std::max(maximum_unchanged_hash_qpc, prior_qpc - unchanged_hash_started_qpc);
            }
            if (producer != nullptr) {
                if (const auto process_sample = sample_process(producer)) process_samples.push_back(*process_sample);
                CloseHandle(producer);
            }

            const double media_seconds = frame_count > 1 && last_timestamp > first_timestamp
                ? static_cast<double>(last_timestamp - first_timestamp) / 10'000'000.0 : 0.0;
            const double fps = media_seconds > 0.0 ? static_cast<double>(frame_count - 1) / media_seconds : 0.0;
            const double wall_seconds = std::chrono::duration<double>(finished - started).count();
            const double maximum_delivery_gap_ms = qpc_frequency.QuadPart == 0 ? 0.0
                : static_cast<double>(maximum_delivery_gap_qpc) * 1000.0 / static_cast<double>(qpc_frequency.QuadPart);
            const double maximum_unchanged_hash_ms = qpc_frequency.QuadPart == 0 ? 0.0
                : static_cast<double>(maximum_unchanged_hash_qpc) * 1000.0 / static_cast<double>(qpc_frequency.QuadPart);
            const double nominal_frame_ms = numerator == 0 ? 1000.0 / 30.0
                : static_cast<double>(denominator) * 1000.0 / static_cast<double>(numerator);
            const double stale_delivery_duration_ms = std::max(0.0, maximum_delivery_gap_ms - nominal_frame_ms);
            // Static content is legitimate, so an unchanged hash is diagnostic
            // evidence only. Visual staleness is classified by correlating the
            // hash timeline with fixture/Zoom state changes in the gate review.
            const double stale_frame_duration_ms = stale_delivery_duration_ms;
            const unsigned effective_warmup_seconds = std::min(options.warmup_seconds, options.duration_seconds / 2U);
            const auto measurement_start = started + std::chrono::seconds{effective_warmup_seconds};
            auto process_baseline = process_samples.begin();
            while (process_baseline != process_samples.end() && process_baseline->wall < measurement_start) ++process_baseline;
            if (process_baseline == process_samples.end() && !process_samples.empty()) process_baseline = process_samples.begin();
            std::vector<double> cpu_percentages;
            const unsigned processor_count = std::max(1U, std::thread::hardware_concurrency());
            for (auto current = process_baseline; current != process_samples.end(); ++current) {
                if (current == process_baseline) continue;
                const auto previous = std::prev(current);
                const auto cpu_delta = current->cpu_100ns - previous->cpu_100ns;
                const auto wall_delta = std::chrono::duration<double>(current->wall - previous->wall).count();
                if (wall_delta > 0.0) {
                    cpu_percentages.push_back(static_cast<double>(cpu_delta) / 10'000'000.0 / wall_delta * 100.0 /
                                              static_cast<double>(processor_count));
                }
            }
            const double average_cpu = cpu_percentages.empty() ? 0.0
                : std::accumulate(cpu_percentages.begin(), cpu_percentages.end(), 0.0) /
                  static_cast<double>(cpu_percentages.size());
            const bool producer_metrics_available = process_baseline != process_samples.end() && std::next(process_baseline) != process_samples.end();
            const std::uint64_t private_growth = producer_metrics_available && process_samples.back().private_bytes > process_baseline->private_bytes
                ? process_samples.back().private_bytes - process_baseline->private_bytes : 0;
            const long long handle_growth = producer_metrics_available
                ? static_cast<long long>(process_samples.back().handles) - static_cast<long long>(process_baseline->handles) : 0;
            const double producer_coverage_seconds = producer_metrics_available
                ? std::chrono::duration<double>(process_samples.back().wall - process_baseline->wall).count() : 0.0;
            const bool passed = qpc_frequency.QuadPart > 0 && frame_count > 1 && read_failures == 0 && timestamp_regressions == 0 &&
                                fps >= options.minimum_fps && fps <= options.maximum_fps &&
                                stale_frame_duration_ms <= options.maximum_stale_ms;

            std::ofstream output(options.output, std::ios::binary | std::ios::trunc);
            if (!output) {
                std::wcerr << L"Could not create output file: " << options.output.wstring() << L'\n';
            } else {
                output << std::fixed << std::setprecision(3)
                       << "{\n  \"schema_version\": 1,\n"
                       << "  \"passed\": " << (passed ? "true" : "false") << ",\n"
                       << "  \"camera_name\": \"" << json_escape(utf8(actual_name)) << "\",\n"
                       << "  \"media_type\": {\"subtype\": \"" << json_escape(guid_string(subtype))
                       << "\", \"subtype_name\": \"" << subtype_name(subtype)
                       << "\", \"width\": " << width << ", \"height\": " << height
                       << ", \"nominal_fps\": " << (denominator == 0 ? 0.0 : static_cast<double>(numerator) / denominator) << "},\n"
                       << "  \"duration_seconds\": " << wall_seconds << ",\n"
                       << "  \"frame_count\": " << frame_count << ",\n"
                       << "  \"measured_fps\": " << fps << ",\n"
                       << "  \"maximum_inter_sample_gap_ms\": " << static_cast<double>(maximum_gap) / 10'000.0 << ",\n"
                       << "  \"maximum_delivery_gap_ms\": " << maximum_delivery_gap_ms << ",\n"
                       << "  \"maximum_unchanged_hash_ms\": " << maximum_unchanged_hash_ms << ",\n"
                       << "  \"stale_frame_duration_ms\": " << stale_frame_duration_ms << ",\n"
                       << "  \"qpc_frequency\": " << qpc_frequency.QuadPart << ",\n"
                       << "  \"qpc_origin\": " << qpc_origin.QuadPart << ",\n"
                       << "  \"timestamp_regressions\": " << timestamp_regressions << ",\n"
                       << "  \"read_failures\": " << read_failures << ",\n"
                       << "  \"frame_hash_changes\": " << hash_changes << ",\n"
                       << "  \"producer_process\": {\"found\": " << (process_samples.empty() ? "false" : "true")
                       << ", \"warmup_seconds\": " << effective_warmup_seconds
                       << ", \"sample_count\": " << process_samples.size()
                       << ", \"measurement_coverage_seconds\": " << producer_coverage_seconds
                       << ", \"average_cpu_percent\": " << average_cpu
                       << ", \"p95_cpu_percent\": " << percentile(cpu_percentages, 0.95)
                       << ", \"private_memory_growth_bytes\": " << private_growth
                       << ", \"handle_growth\": " << handle_growth << "},\n"
                       << "  \"hash_samples\": [";
                for (std::size_t index = 0; index < hash_samples.size(); ++index) {
                    if (index != 0) output << ',';
                    output << "\n    {\"frame\": " << hash_samples[index].frame
                           << ", \"timestamp_100ns\": " << hash_samples[index].timestamp
                           << ", \"qpc_ticks\": " << hash_samples[index].qpc_ticks
                           << ", \"fnv1a64\": \"" << std::hex << std::setw(16) << std::setfill('0') << hash_samples[index].hash
                           << std::dec << std::setfill(' ') << "\"}";
                }
                if (!hash_samples.empty()) output << '\n';
                output << "  ]\n}\n";
                if (!output) {
                    std::wcerr << L"Writing the output file failed.\n";
                } else {
                    std::wcout << L"Captured " << frame_count << L" frames at " << fps << L" fps; result="
                               << (passed ? L"pass" : L"fail") << L"; output=" << options.output.wstring() << L'\n';
                    exit_code = passed ? 0 : 1;
                }
            }
        } else {
            std::wcerr << L"Could not create a Media Foundation source reader.\n";
        }
        source->Shutdown();
    }

    MFShutdown();
    CoUninitialize();
    return exit_code;
}
