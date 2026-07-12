cmake_minimum_required(VERSION 3.24)
project(AutomaticScreenCamera VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
if(MSVC)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

option(ASC_BUILD_TESTS "Build portable unit tests" ON)
option(ASC_BUILD_WINDOWS_APP "Build the Windows desktop application" ${WIN32})

add_library(asc_core STATIC
    src/core/config.cpp
    src/core/controller.cpp
    src/core/event_log.cpp
    src/core/image.cpp
    src/core/detector.cpp
    src/core/decision_engine.cpp
    src/core/monitor_tracker.cpp
    src/core/transition.cpp
)
target_include_directories(asc_core PUBLIC include)
target_compile_features(asc_core PUBLIC cxx_std_20)
if(WIN32)
    target_compile_definitions(asc_core PRIVATE WIN32_LEAN_AND_MEAN NOMINMAX UNICODE _UNICODE)
endif()
if(MSVC)
    target_compile_options(asc_core PRIVATE /W4 /permissive- /EHsc)
else()
    target_compile_options(asc_core PRIVATE -Wall -Wextra -Wpedantic -Wconversion)
endif()

if(ASC_BUILD_TESTS)
    enable_testing()
    add_executable(asc_core_tests tests/core_tests.cpp)
    target_link_libraries(asc_core_tests PRIVATE asc_core)
    add_test(NAME asc_core_tests COMMAND asc_core_tests)
endif()

if(ASC_BUILD_WINDOWS_APP)
    if(NOT WIN32)
        message(FATAL_ERROR "ASC_BUILD_WINDOWS_APP requires a Windows toolchain")
    endif()
    add_subdirectory(src/windows)
endif()
