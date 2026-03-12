/**
 * Test: NA message client
 *
 * 1. Read server address from file
 * 2. Initialize (not listening)
 * 3. Look up server address
 * 4. Send unexpected message to server
 * 5. Send expected message to server
 * 6. Recv expected reply from server
 * 7. Finalize
 *
 * Usage: test_abc_msg_client [-p <protocol>]
 */

#include <na.h>
#include <na_types.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct client_state {
    na_class_t *na_class;
    na_context_t *context;

    bool send_unexpected_done;
    bool send_expected_done;

    bool recv_expected_done;
    char recv_buf[4096];
    size_t recv_size;
};

static void
send_unexpected_cb(const struct na_cb_info *info)
{
    struct client_state *state = (struct client_state *) info->arg;

    if (info->ret != NA_SUCCESS)
        fprintf(stderr, "Client: unexpected send failed: %d\n", info->ret);
    state->send_unexpected_done = true;
}

static void
send_expected_cb(const struct na_cb_info *info)
{
    struct client_state *state = (struct client_state *) info->arg;

    if (info->ret != NA_SUCCESS)
        fprintf(stderr, "Client: expected send failed: %d\n", info->ret);
    state->send_expected_done = true;
}

static void
recv_expected_cb(const struct na_cb_info *info)
{
    struct client_state *state = (struct client_state *) info->arg;

    if (info->ret != NA_SUCCESS)
        fprintf(stderr, "Client: expected recv failed: %d\n", info->ret);
    state->recv_size = info->info.recv_expected.actual_buf_size;
    state->recv_expected_done = true;
}

static na_return_t
progress_loop(struct client_state *state, bool *flag)
{
    unsigned int count;
    unsigned int total_triggered;
    na_return_t ret;
    int timeout_count = 0;

    while (!(*flag)) {
        ret = NA_Poll(state->na_class, state->context, &count);
        if (ret != NA_SUCCESS && ret != NA_TIMEOUT)
            return ret;

        do {
            total_triggered = 0;
            ret = NA_Trigger(state->context, 1, &total_triggered);
        } while (total_triggered > 0 && ret == NA_SUCCESS);

        if (!(*flag)) {
            usleep(1000);
            timeout_count++;
            if (timeout_count > 30000) {
                fprintf(stderr, "Client: timeout waiting\n");
                return NA_TIMEOUT;
            }
        }
    }

    return NA_SUCCESS;
}

int
main(int argc, char *argv[])
{
    const char *protocol = "abc";
    char info_string[256];
    struct client_state state;
    const char *addr_file = "na_test_addr.txt";
    char server_addr_str[256];
    na_addr_t *server_addr = NULL;
    na_op_id_t *op_id = NULL;
    na_return_t ret;
    int rc = EXIT_FAILURE;
    int retry;
    int opt;

    while ((opt = getopt(argc, argv, "p:")) != -1) {
        switch (opt) {
            case 'p':
                protocol = optarg;
                break;
            default:
                fprintf(stderr, "Usage: %s [-p protocol]\n", argv[0]);
                return EXIT_FAILURE;
        }
    }

    snprintf(info_string, sizeof(info_string), "%s://", protocol);
    memset(&state, 0, sizeof(state));

    /* Wait for server address file */
    printf("Client: waiting for server address...\n");
    for (retry = 0; retry < 100; retry++) {
        FILE *f = fopen(addr_file, "r");
        if (f) {
            if (fgets(server_addr_str, sizeof(server_addr_str), f) != NULL &&
                strlen(server_addr_str) > 0) {
                fclose(f);
                break;
            }
            fclose(f);
        }
        usleep(100000); /* 100ms */
    }
    if (retry >= 100) {
        fprintf(stderr, "Client: timed out waiting for server address\n");
        goto done;
    }

    printf("Client: server address = %s\n", server_addr_str);

    /* Initialize */
    printf("Client: initializing...\n");
    state.na_class = NA_Initialize(info_string, false);
    if (state.na_class == NULL) {
        fprintf(stderr, "Client: NA_Initialize failed\n");
        goto done;
    }

    state.context = NA_Context_create(state.na_class);
    if (state.context == NULL) {
        fprintf(stderr, "Client: NA_Context_create failed\n");
        goto cleanup_class;
    }

    /* Look up server */
    printf("Client: looking up %s...\n", server_addr_str);
    ret = NA_Addr_lookup(state.na_class, server_addr_str, &server_addr);
    if (ret != NA_SUCCESS) {
        fprintf(stderr, "Client: NA_Addr_lookup failed: %d\n", ret);
        goto cleanup_ctx;
    }

    /* Give transport a moment to establish connection */
    usleep(200000);

    /* Step 1: Send unexpected message */
    printf("Client: sending unexpected message...\n");
    {
        const char *msg = "Hello from client";
        op_id = NA_Op_create(state.na_class, NA_OP_SINGLE);
        ret = NA_Msg_send_unexpected(state.na_class, state.context,
            send_unexpected_cb, &state, msg, strlen(msg), NULL,
            server_addr, 0, 100, op_id);
        if (ret != NA_SUCCESS) {
            fprintf(stderr, "Client: send_unexpected failed: %d\n", ret);
            goto cleanup_op;
        }

        ret = progress_loop(&state, &state.send_unexpected_done);
        if (ret != NA_SUCCESS)
            goto cleanup_op;
    }
    printf("Client: unexpected send complete\n");
    NA_Op_destroy(state.na_class, op_id);

    /* Small delay for server to process */
    usleep(100000);

    /* Step 2: Send expected message */
    printf("Client: sending expected message...\n");
    {
        const char *msg = "Expected from client";
        op_id = NA_Op_create(state.na_class, NA_OP_SINGLE);
        ret = NA_Msg_send_expected(state.na_class, state.context,
            send_expected_cb, &state, msg, strlen(msg), NULL,
            server_addr, 0, 200, op_id);
        if (ret != NA_SUCCESS) {
            fprintf(stderr, "Client: send_expected failed: %d\n", ret);
            goto cleanup_op;
        }

        ret = progress_loop(&state, &state.send_expected_done);
        if (ret != NA_SUCCESS)
            goto cleanup_op;
    }
    printf("Client: expected send complete\n");
    NA_Op_destroy(state.na_class, op_id);

    /* Step 3: Recv expected reply from server */
    printf("Client: waiting for expected reply...\n");
    op_id = NA_Op_create(state.na_class, NA_OP_SINGLE);
    ret = NA_Msg_recv_expected(state.na_class, state.context,
        recv_expected_cb, &state, state.recv_buf, sizeof(state.recv_buf),
        NULL, server_addr, 0, 300, op_id);
    if (ret != NA_SUCCESS) {
        fprintf(stderr, "Client: recv_expected failed: %d\n", ret);
        goto cleanup_op;
    }

    ret = progress_loop(&state, &state.recv_expected_done);
    if (ret != NA_SUCCESS)
        goto cleanup_op;

    printf("Client: received expected reply (size=%zu): \"%.*s\"\n",
        state.recv_size, (int) state.recv_size, state.recv_buf);

    if (strncmp(state.recv_buf, "Reply from server", 17) != 0) {
        fprintf(stderr, "Client: reply mismatch!\n");
        goto cleanup_op;
    }

    rc = EXIT_SUCCESS;
    printf("Client: === PASS ===\n");

cleanup_op:
    NA_Op_destroy(state.na_class, op_id);
    if (server_addr)
        NA_Addr_free(state.na_class, server_addr);
cleanup_ctx:
    NA_Context_destroy(state.na_class, state.context);
cleanup_class:
    NA_Finalize(state.na_class);
done:
    return rc;
}
