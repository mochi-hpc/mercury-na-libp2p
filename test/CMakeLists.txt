# Test executables for NA plugin
#
# After renaming the plugin with rename-plugin.sh, the test files and
# targets are updated automatically (the rename script handles the
# test_abc_* -> test_<name>_* renaming).

# Make test headers available
include_directories(${CMAKE_CURRENT_SOURCE_DIR})

# Common test library (adapted from Mercury's testing infrastructure)
add_library(na_test_common_lib STATIC
    na_test_getopt.c
    na_test.c
    na_test_common.c
)
target_link_libraries(na_test_common_lib PRIVATE na mercury_util)

# Init/finalize smoke test
add_executable(test_abc_init test_abc_init.c)
target_link_libraries(test_abc_init PRIVATE na)

# Message test - server
add_executable(test_abc_msg_server test_abc_msg_server.c)
target_link_libraries(test_abc_msg_server PRIVATE na)

# Message test - client
add_executable(test_abc_msg_client test_abc_msg_client.c)
target_link_libraries(test_abc_msg_client PRIVATE na)

# Integration tests (adapted from Mercury's Testing/unit/na/)
# Lookup test - server
add_executable(test_lookup_server test_lookup_server.c)
target_link_libraries(test_lookup_server PRIVATE na_test_common_lib na mercury_util)

# Lookup test - client
add_executable(test_lookup test_lookup.c)
target_link_libraries(test_lookup PRIVATE na_test_common_lib na mercury_util)

# ============================================================
# HG-level unit tests (adapted from Mercury's Testing/unit/hg/)
# ============================================================

# HG test common library (mercury_test.c + mercury_unit.c + mercury_rpc_cb.c)
add_library(hg_test_common_lib STATIC
    mercury_test.c
    mercury_unit.c
    mercury_rpc_cb.c
)
target_link_libraries(hg_test_common_lib PUBLIC na_test_common_lib mercury mercury_util)

# HG test server
add_executable(test_hg_server test_hg_server.c)
target_link_libraries(test_hg_server PRIVATE hg_test_common_lib)

# HG RPC test (client)
add_executable(test_rpc test_rpc.c)
target_link_libraries(test_rpc PRIVATE hg_test_common_lib)

# HG Bulk test (client)
add_executable(test_bulk test_bulk.c)
target_link_libraries(test_bulk PRIVATE hg_test_common_lib)

# HG Proc test (standalone, no server needed)
add_executable(test_proc test_proc.c)
target_link_libraries(test_proc PRIVATE hg_test_common_lib)

# ============================================================
# CTest definitions
# ============================================================

set(TEST_DRIVER ${CMAKE_CURRENT_SOURCE_DIR}/test_driver.sh)

# Standalone tests
add_test(NAME abc_init COMMAND test_abc_init -p abc)
add_test(NAME abc_proc COMMAND test_proc)

# Server+client tests
add_test(NAME abc_msg
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_abc_msg_server> -p abc --
        $<TARGET_FILE:test_abc_msg_client> -p abc)

add_test(NAME abc_lookup
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_lookup_server> -p abc --
        $<TARGET_FILE:test_lookup> -p abc)

add_test(NAME abc_rpc
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_hg_server> -p abc --
        $<TARGET_FILE:test_rpc> -p abc)

add_test(NAME abc_bulk
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_hg_server> -p abc --
        $<TARGET_FILE:test_bulk> -p abc)

# All tests need NA_PLUGIN_PATH to find the Rust plugin (.so is in cargo-build/release/)
set(_na_plugin_dir "${CMAKE_CURRENT_BINARY_DIR}/../cargo-build/release")
set_tests_properties(abc_init abc_proc abc_msg abc_lookup abc_rpc abc_bulk
    PROPERTIES ENVIRONMENT "NA_PLUGIN_PATH=${_na_plugin_dir}")
