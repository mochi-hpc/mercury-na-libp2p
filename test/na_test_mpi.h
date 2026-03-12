/**
 * MPI stub for standalone (non-MPI) testing.
 * Provides the same interface as Mercury's na_test_mpi.h but without MPI.
 */

#ifndef NA_TEST_MPI_H
#define NA_TEST_MPI_H

#include "mercury_test_config.h"
#include "na.h"

#include <stdbool.h>
#include <string.h>

/*************************************/
/* Public Type and Struct Definition */
/*************************************/

union na_test_mpi_comm {
    int mpich;
};

struct na_test_mpi_info {
    union na_test_mpi_comm comm;
    int rank;
    int size;
    bool mpi_no_finalize;
};

/*********************/
/* Public Prototypes */
/*********************/

static inline na_return_t
na_test_mpi_init(struct na_test_mpi_info *mpi_info, bool NA_UNUSED listen,
    bool NA_UNUSED use_threads, bool NA_UNUSED mpi_static)
{
    memset(mpi_info, 0, sizeof(*mpi_info));
    mpi_info->rank = 0;
    mpi_info->size = 1;
    return NA_SUCCESS;
}

static inline void
na_test_mpi_finalize(struct na_test_mpi_info NA_UNUSED *mpi_info)
{
}

static inline na_return_t
na_test_mpi_barrier(const struct na_test_mpi_info NA_UNUSED *mpi_info)
{
    return NA_SUCCESS;
}

static inline na_return_t
na_test_mpi_barrier_world(void)
{
    return NA_SUCCESS;
}

static inline na_return_t
na_test_mpi_bcast(const struct na_test_mpi_info NA_UNUSED *mpi_info,
    void NA_UNUSED *buffer, size_t NA_UNUSED size, int NA_UNUSED root)
{
    return NA_SUCCESS;
}

static inline na_return_t
na_test_mpi_gather(const struct na_test_mpi_info NA_UNUSED *mpi_info,
    const void NA_UNUSED *sendbuf, size_t NA_UNUSED sendcount,
    void NA_UNUSED *recvbuf, size_t NA_UNUSED recvcount, int NA_UNUSED root)
{
    return NA_SUCCESS;
}

#endif /* NA_TEST_MPI_H */
