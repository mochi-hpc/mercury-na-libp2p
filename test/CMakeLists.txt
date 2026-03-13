# Test executables for NA plugin
#
# After renaming the plugin with rename-plugin.sh, the test files and
# targets are updated automatically (the rename script handles the
# test_libp2p_* -> test_<name>_* renaming).

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
add_executable(test_libp2p_init test_libp2p_init.c)
target_link_libraries(test_libp2p_init PRIVATE na)

# Message test - server
add_executable(test_libp2p_msg_server test_libp2p_msg_server.c)
target_link_libraries(test_libp2p_msg_server PRIVATE na)

# Message test - client
add_executable(test_libp2p_msg_client test_libp2p_msg_client.c)
target_link_libraries(test_libp2p_msg_client PRIVATE na)

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
add_test(NAME libp2p_init COMMAND test_libp2p_init -p libp2p)
add_test(NAME libp2p_proc COMMAND test_proc)

# Server+client tests
add_test(NAME libp2p_msg
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_libp2p_msg_server> -p libp2p --
        $<TARGET_FILE:test_libp2p_msg_client> -p libp2p)

add_test(NAME libp2p_lookup
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_lookup_server> -p libp2p --
        $<TARGET_FILE:test_lookup> -p libp2p)

add_test(NAME libp2p_rpc
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_hg_server> -p libp2p --
        $<TARGET_FILE:test_rpc> -p libp2p)

add_test(NAME libp2p_bulk
    COMMAND ${TEST_DRIVER}
        $<TARGET_FILE:test_hg_server> -p libp2p --
        $<TARGET_FILE:test_bulk> -p libp2p)

# All tests need NA_PLUGIN_PATH to find the Rust plugin (.so is in cargo-build/release/)
set(_na_plugin_dir "${CMAKE_CURRENT_BINARY_DIR}/../cargo-build/release")
set_tests_properties(libp2p_init libp2p_proc libp2p_msg libp2p_lookup libp2p_rpc libp2p_bulk
    PROPERTIES ENVIRONMENT "NA_PLUGIN_PATH=${_na_plugin_dir}")

# ============================================================
# Relay test infrastructure
# ============================================================

# Relay server binary is built by the parent CMakeLists.txt
set(_relay_server_bin "${CMAKE_CURRENT_BINARY_DIR}/../cargo-build/release/mercury-na-relay-server")

# Relay message test executables
add_executable(test_libp2p_relay_server test_libp2p_relay_server.c)
target_link_libraries(test_libp2p_relay_server PRIVATE na)

add_executable(test_libp2p_relay_client test_libp2p_relay_client.c)
target_link_libraries(test_libp2p_relay_client PRIVATE na)

# 3-process relay message test
set(TEST_RELAY_DRIVER ${CMAKE_CURRENT_SOURCE_DIR}/test_relay_driver.sh)

add_test(NAME libp2p_relay_msg
    COMMAND ${TEST_RELAY_DRIVER}
        ${_relay_server_bin} --port 0 --secret-key-seed 1 --addr-file relay_addr.txt --
        $<TARGET_FILE:test_libp2p_relay_server> -p "tcp,relay" --
        $<TARGET_FILE:test_libp2p_relay_client> -p "tcp,relay")
add_dependencies(test_libp2p_relay_server mercury_na_relay_server)
add_dependencies(test_libp2p_relay_client mercury_na_relay_server)

set_tests_properties(libp2p_relay_msg
    PROPERTIES
        ENVIRONMENT "NA_PLUGIN_PATH=${_na_plugin_dir}"
        TIMEOUT 60)
