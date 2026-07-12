add_library(AutomaticScreenCameraSource SHARED
    dllmain.cpp
    activation.cpp
    media_source.cpp
    media_stream.cpp
    ../shared_frame.cpp
)
target_include_directories(AutomaticScreenCameraSource PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/..
    ${PROJECT_SOURCE_DIR}/include
)
target_compile_definitions(AutomaticScreenCameraSource PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX _WIN32_WINNT=0x0A00)
target_compile_options(AutomaticScreenCameraSource PRIVATE /W4 /permissive- /EHsc)
target_link_libraries(AutomaticScreenCameraSource PRIVATE asc_core d3d11 dxgi mf mfplat mfuuid mfsensorgroup ole32 advapi32)
set_target_properties(AutomaticScreenCameraSource PROPERTIES OUTPUT_NAME "AutomaticScreenCameraSource")
