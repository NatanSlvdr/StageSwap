add_executable(AutomaticScreenCamera WIN32
    app.rc
    main.cpp
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
    virtual_camera.cpp
    tray_window.cpp
)

target_include_directories(AutomaticScreenCamera PRIVATE ${CMAKE_CURRENT_SOURCE_DIR} ${PROJECT_SOURCE_DIR}/include)
target_compile_definitions(AutomaticScreenCamera PRIVATE
    UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX
    WINVER=0x0A00 _WIN32_WINNT=0x0A00
)
asc_enable_project_warnings(AutomaticScreenCamera)
target_compile_options(AutomaticScreenCamera PRIVATE /await:strict $<$<CONFIG:Release>:/Zi>)
target_link_options(AutomaticScreenCamera PRIVATE $<$<CONFIG:Release>:/DEBUG:FULL>)
target_link_libraries(AutomaticScreenCamera PRIVATE
    asc_core
    d3d11 dxgi d3dcompiler
    mf mfplat mfreadwrite mfuuid mfsensorgroup
    windowsapp runtimeobject shcore setupapi cfgmgr32
    comctl32 comdlg32 shell32 ole32 user32 gdi32 advapi32 windowscodecs wtsapi32
)

add_subdirectory(media_source)
set_target_properties(AutomaticScreenCamera PROPERTIES OUTPUT_NAME "AutomaticScreenCamera")
