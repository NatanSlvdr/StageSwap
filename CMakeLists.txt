cmake_minimum_required(VERSION 3.24)
project(AutomaticScreenCamera VERSION 0.1.0 LANGUAGES CXX)

set(ASC_PACKAGE_VERSION "${PROJECT_VERSION}" CACHE STRING "Version embedded in packaged Windows artifacts")
set(ASC_SOURCE_REVISION "unknown" CACHE STRING "Source revision embedded in packaged Windows artifacts")
if(NOT ASC_PACKAGE_VERSION MATCHES "^([0-9]+)\\.([0-9]+)\\.([0-9]+)(-[0-9A-Za-z.-]+)?$")
    message(FATAL_ERROR "ASC_PACKAGE_VERSION must be a semantic version such as 1.2.3 or 1.2.3-beta.1")
endif()
set(ASC_VERSION_MAJOR "${CMAKE_MATCH_1}")
set(ASC_VERSION_MINOR "${CMAKE_MATCH_2}")
set(ASC_VERSION_PATCH "${CMAKE_MATCH_3}")

if(WIN32)
    set(ASC_VERSION_FILE_TYPE VFT_DLL)
    set(ASC_VERSION_DESCRIPTION "Automatic Screen Camera Media Source")
    set(ASC_VERSION_INTERNAL_NAME "AutomaticScreenCameraSource")
    set(ASC_VERSION_ORIGINAL_FILENAME "AutomaticScreenCameraSource.dll")
    set(ASC_MEDIA_SOURCE_VERSION_RESOURCE "${CMAKE_CURRENT_BINARY_DIR}/AutomaticScreenCameraSource.version.rc")
    configure_file("${CMAKE_CURRENT_SOURCE_DIR}/cmake/version.rc.in" "${ASC_MEDIA_SOURCE_VERSION_RESOURCE}" @ONLY)
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
if(MSVC)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

option(ASC_BUILD_TESTS "Build portable unit tests" ON)
option(ASC_BUILD_WINDOWS_APP "Build the Windows desktop application" ${WIN32})
option(ASC_BUILD_WINDOWS_TEST_TOOLS "Build Windows-only release verification tools" OFF)
option(ASC_WARNINGS_AS_ERRORS "Treat compiler warnings as errors" OFF)
option(ASC_ENABLE_SANITIZERS "Enable AddressSanitizer and UndefinedBehaviorSanitizer" OFF)
option(ASC_ENABLE_THREAD_SANITIZER "Enable ThreadSanitizer" OFF)

function(asc_enable_project_warnings target)
    if(MSVC)
        target_compile_options(${target} PRIVATE /W4 /permissive- /EHsc)
        if(ASC_WARNINGS_AS_ERRORS)
            target_compile_options(${target} PRIVATE /WX)
        endif()
    else()
        target_compile_options(${target} PRIVATE -Wall -Wextra -Wpedantic -Wconversion)
        if(ASC_WARNINGS_AS_ERRORS)
            target_compile_options(${target} PRIVATE -Werror)
        endif()
    endif()
endfunction()

function(asc_enable_sanitizers target)
    if(NOT ASC_ENABLE_SANITIZERS AND NOT ASC_ENABLE_THREAD_SANITIZER)
        return()
    endif()

    if(ASC_ENABLE_SANITIZERS AND ASC_ENABLE_THREAD_SANITIZER)
        message(FATAL_ERROR "Address/Undefined sanitizers cannot be combined with ThreadSanitizer")
    endif()

    if(CMAKE_CXX_COMPILER_ID MATCHES "Clang|GNU")
        if(ASC_ENABLE_THREAD_SANITIZER)
            set(sanitizer_flags -fsanitize=thread -fno-omit-frame-pointer)
        else()
            set(sanitizer_flags -fsanitize=address,undefined -fno-omit-frame-pointer)
        endif()
        target_compile_options(${target} PRIVATE ${sanitizer_flags})
        get_target_property(target_type ${target} TYPE)
        if(NOT target_type STREQUAL "STATIC_LIBRARY")
            target_link_options(${target} PRIVATE ${sanitizer_flags})
        endif()
    else()
        message(FATAL_ERROR "Requested sanitizers require a Clang or GCC toolchain")
    endif()
endfunction()

add_library(asc_core STATIC
    src/core/config.cpp
    src/core/controller.cpp
    src/core/event_log.cpp
    src/core/frame.cpp
    src/core/image.cpp
    src/core/detector.cpp
    src/core/decision_engine.cpp
    src/core/monitor_tracker.cpp
    src/core/transition.cpp
)
target_include_directories(asc_core PUBLIC include)
target_compile_features(asc_core PUBLIC cxx_std_20)
asc_enable_project_warnings(asc_core)
asc_enable_sanitizers(asc_core)
if(WIN32)
    target_compile_definitions(asc_core PRIVATE WIN32_LEAN_AND_MEAN NOMINMAX UNICODE _UNICODE)
endif()

if(ASC_BUILD_TESTS)
    enable_testing()
    find_package(Threads REQUIRED)
    add_executable(asc_core_tests tests/core_tests.cpp)
    target_link_libraries(asc_core_tests PRIVATE asc_core Threads::Threads)
    asc_enable_project_warnings(asc_core_tests)
    asc_enable_sanitizers(asc_core_tests)
    add_test(NAME asc_core_tests COMMAND asc_core_tests)
    set_tests_properties(asc_core_tests PROPERTIES TIMEOUT 180)

    add_executable(asc_ui_presentation_tests
        tests/ui_presentation_tests.cpp
        src/windows/status_presentation.cpp
    )
    target_include_directories(asc_ui_presentation_tests PRIVATE include src/windows)
    target_link_libraries(asc_ui_presentation_tests PRIVATE asc_core Threads::Threads)
    asc_enable_project_warnings(asc_ui_presentation_tests)
    asc_enable_sanitizers(asc_ui_presentation_tests)
    add_test(NAME asc_ui_presentation_tests COMMAND asc_ui_presentation_tests)
    set_tests_properties(asc_ui_presentation_tests PROPERTIES TIMEOUT 30)
endif()

if(ASC_BUILD_WINDOWS_APP)
    if(NOT WIN32)
        message(FATAL_ERROR "ASC_BUILD_WINDOWS_APP requires a Windows toolchain")
    endif()
    add_subdirectory(src/windows)
endif()

if(WIN32 AND ASC_BUILD_WINDOWS_TEST_TOOLS AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/tests/windows/CMakeLists.txt")
    add_subdirectory(tests/windows)
endif()
