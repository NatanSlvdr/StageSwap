set(ASC_WINDOWS_APP_SOURCES
    app.rc
    main.cpp
    deployment.cpp
    app.cpp
    d3d_device.cpp
    device_enumerator.cpp
    screen_capture.cpp
    video_input.cpp
    compositor.cpp
    shared_frame.cpp
    reference_store.cpp
    settings_window.cpp
    preview_window.cpp
    status_presentation.cpp
    ui_helpers.cpp
    virtual_camera.cpp
    tray_window.cpp
)

add_subdirectory(media_source)

function(asc_configure_windows_app target output_name version_resource)
    add_executable(${target} WIN32 ${ASC_WINDOWS_APP_SOURCES} ${version_resource})
    target_include_directories(${target} PRIVATE ${CMAKE_CURRENT_SOURCE_DIR} ${PROJECT_SOURCE_DIR}/include)
    target_compile_definitions(${target} PRIVATE
        UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX
        WINVER=0x0A00 _WIN32_WINNT=0x0A00
    )
    asc_enable_project_warnings(${target})
    target_compile_options(${target} PRIVATE /await:strict $<$<CONFIG:Release>:/Zi>)
    target_link_options(${target} PRIVATE /MANIFEST:NO $<$<CONFIG:Release>:/DEBUG:FULL>)
    target_link_libraries(${target} PRIVATE
        asc_core
        d3d11 dxgi d3dcompiler
        mf mfplat mfreadwrite mfuuid mfsensorgroup
        windowsapp runtimeobject shcore setupapi cfgmgr32 bcrypt
        comctl32 comdlg32 shell32 ole32 user32 gdi32 advapi32 windowscodecs wtsapi32
    )
    set_target_properties(${target} PROPERTIES OUTPUT_NAME "${output_name}")
endfunction()

set(ASC_VERSION_FILE_TYPE VFT_APP)
set(ASC_VERSION_DESCRIPTION "Automatic Screen Camera")
set(ASC_VERSION_INTERNAL_NAME "AutomaticScreenCamera")
set(ASC_VERSION_ORIGINAL_FILENAME "AutomaticScreenCamera.exe")
set(app_version_resource "${CMAKE_CURRENT_BINARY_DIR}/AutomaticScreenCamera.version.rc")
configure_file("${PROJECT_SOURCE_DIR}/cmake/version.rc.in" "${app_version_resource}" @ONLY)
asc_configure_windows_app(AutomaticScreenCamera AutomaticScreenCamera "${app_version_resource}")

set(portable_resource "${CMAKE_CURRENT_BINARY_DIR}/AutomaticScreenCameraPortable.payload.rc")
set(portable_header "${CMAKE_CURRENT_BINARY_DIR}/portable_payload.hpp")
add_custom_command(
    OUTPUT "${portable_resource}" "${portable_header}"
    COMMAND ${CMAKE_COMMAND}
        "-DINPUT=$<TARGET_FILE:AutomaticScreenCameraSource>"
        "-DRESOURCE_OUTPUT=${portable_resource}"
        "-DHEADER_OUTPUT=${portable_header}"
        "-DVERSION=${ASC_PACKAGE_VERSION}"
        "-DREVISION=${ASC_SOURCE_REVISION}"
        "-DCONFIGURATION=$<CONFIG>"
        "-DWINDOWS_SDK=${CMAKE_SYSTEM_VERSION}"
        -P "${PROJECT_SOURCE_DIR}/cmake/GeneratePortableResource.cmake"
    DEPENDS AutomaticScreenCameraSource "${PROJECT_SOURCE_DIR}/cmake/GeneratePortableResource.cmake"
    VERBATIM
)
set_source_files_properties("${portable_resource}" "${portable_header}" PROPERTIES GENERATED TRUE)

set(ASC_VERSION_DESCRIPTION "Automatic Screen Camera Portable")
set(ASC_VERSION_INTERNAL_NAME "AutomaticScreenCameraPortable")
set(ASC_VERSION_ORIGINAL_FILENAME "AutomaticScreenCameraPortable.exe")
set(portable_version_resource "${CMAKE_CURRENT_BINARY_DIR}/AutomaticScreenCameraPortable.version.rc")
configure_file("${PROJECT_SOURCE_DIR}/cmake/version.rc.in" "${portable_version_resource}" @ONLY)
asc_configure_windows_app(AutomaticScreenCameraPortable AutomaticScreenCameraPortable "${portable_version_resource}")
target_sources(AutomaticScreenCameraPortable PRIVATE "${portable_resource}" "${portable_header}")
target_include_directories(AutomaticScreenCameraPortable PRIVATE "${CMAKE_CURRENT_BINARY_DIR}")
target_compile_definitions(AutomaticScreenCameraPortable PRIVATE ASC_PORTABLE_BUILD)
