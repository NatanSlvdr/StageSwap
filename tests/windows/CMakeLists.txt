add_executable(asc_test_screen_fixture test_screen_fixture.cpp)
target_compile_definitions(asc_test_screen_fixture PRIVATE
    UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX
)
asc_enable_project_warnings(asc_test_screen_fixture)
target_link_libraries(asc_test_screen_fixture PRIVATE user32 gdi32)

add_executable(asc_mf_output_probe mf_output_probe.cpp)
target_compile_definitions(asc_mf_output_probe PRIVATE
    UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX
)
asc_enable_project_warnings(asc_mf_output_probe)
target_link_libraries(asc_mf_output_probe PRIVATE
    mf mfplat mfreadwrite mfuuid ole32 psapi
)

set_target_properties(asc_test_screen_fixture asc_mf_output_probe PROPERTIES
    FOLDER "Reliability Tests"
)

if(ASC_BUILD_TESTS)
    add_test(NAME asc_test_screen_fixture_help COMMAND asc_test_screen_fixture --help)
    add_test(NAME asc_mf_output_probe_help COMMAND asc_mf_output_probe --help)
    set_tests_properties(asc_test_screen_fixture_help asc_mf_output_probe_help PROPERTIES LABELS "windows;reliability")
endif()
