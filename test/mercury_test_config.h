/**
 * Test configuration for ZMQ NA plugin (standalone, no MPI).
 * Adapted from Mercury's mercury_test_config.h.in.
 */

#ifndef MERCURY_TEST_CONFIG_H
#define MERCURY_TEST_CONFIG_H

/* Temp directory for config files */
#define HG_TEST_TEMP_DIRECTORY "/tmp"

/* Start msg - used by test driver to detect server readiness */
#define HG_TEST_SERVER_START_MSG "# Waiting for client"
#include <stdio.h>
#define HG_TEST_READY_MSG()                                                    \
    do {                                                                       \
        printf(HG_TEST_SERVER_START_MSG "\n");                                 \
        fflush(stdout);                                                        \
    } while (0)

/* Timeout (in seconds) */
#define HG_TEST_TIMEOUT 240

/* Number of threads */
#define HG_TEST_NUM_THREADS_DEFAULT (8)

#endif /* MERCURY_TEST_CONFIG_H */
