/**
 * Test: NA message server
 *
 * 1. Initialize as listener
 * 2. Write self address to file for client
 * 3. Post unexpected recv, wait for client's unexpected message
 * 4. Post expected recv from client, wait
 * 5. Send expected reply to client
 * 6. Finalize
 *
 * Usage: test_libp2p_msg_server [-p <protocol>]
 */

#include <na.h>
#include <na_types.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct server_state {
    na_class_t *na_class;
    na_context_t *context;

    /* Unexpected recv result */
    bool unexpected_done;
    na_addr_t *client_addr;
    na_tag_t recv_tag;
    char recv_buf[4096];
    size_t recv_size;

    /* Expected recv result */
    bool expected_done;
    char expected_buf[4096];
    size_t expected_size;

    /* Send result */
    bool send_done;
};

static void
unexpected_recv_cb(const struct na_cb_info *info)
{
    struct server_state *state = (struct server_state *) info->arg;

    if (info->ret != NA_SUCCESS) {
        fprintf(stderr, "Server: unexpected recv failed: %d\n", info->ret);
        state->unexpected_done = true;
        return;
    }

    state->recv_size = info->info.recv_unexpected.actual_buf_size;
    state->client_addr = info->info.recv_unexpected.source;
    state->recv_tag = info->info.recv_unexpected.tag;
    state->unexpected_done = true;
}

static void
expected_recv_cb(const struct na_cb_info *info)
{
    struct server_state *state = (struct server_state *) info->arg;

    if (info->ret != NA_SUCCESS) {
        fprintf(stderr, "Server: expected recv failed: %d\n", info->ret);
    }
    state->expected_size = info->info.recv_expected.actual_buf_size;
    state->expected_done = true;
}

static void
send_cb(const struct na_cb_info *info)
{
    struct server_state *state = (struct server_state *) info->arg;

    if (info->ret != NA_SUCCESS)
        fprintf(stderr, "Server: send failed: %d\n", info->ret);
    state->send_done = true;
}

static na_return_t
progress_loop(struct server_state *state, bool *flag)
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
            usleep(1000); /* 1ms */
            timeout_count++;
            if (timeout_count > 30000) { /* 30s */
                fprintf(stderr, "Server: timeout waiting for operation\n");
                return NA_TIMEOUT;
            }
        }
    }

    return NA_SUCCESS;
}

int
main(int argc, char *argv[])
{
    const char *protocol = "libp2p";
    char info_string[256];
    struct server_state state;
    na_op_id_t *op_id = NULL;
    na_addr_t *self_addr = NULL;
    char addr_str[256];
    size_t addr_str_size = sizeof(addr_str);
    const char *addr_file = "na_test_addr.txt";
    na_return_t ret;
    int rc = EXIT_FAILURE;
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

    printf("Server: initializing...\n");
    state.na_class = NA_Initialize(info_string, true);
    if (state.na_class == NULL) {
        fprintf(stderr, "Server: NA_Initialize failed\n");
        goto done;
    }

    state.context = NA_Context_create(state.na_class);
    if (state.context == NULL) {
        fprintf(stderr, "Server: NA_Context_create failed\n");
        goto cleanup_class;
    }

    /* Get self address and write to file */
    ret = NA_Addr_self(state.na_class, &self_addr);
    if (ret != NA_SUCCESS) {
        fprintf(stderr, "Server: NA_Addr_self failed\n");
        goto cleanup_ctx;
    }

    ret = NA_Addr_to_string(state.na_class, addr_str, &addr_str_size,
        self_addr);
    if (ret != NA_SUCCESS) {
        fprintf(stderr, "Server: NA_Addr_to_string failed\n");
        goto cleanup_self;
    }
    NA_Addr_free(state.na_class, self_addr);
    self_addr = NULL;

    printf("Server: listening at %s\n", addr_str);

    /* Write address file */
    {
        FILE *f = fopen(addr_file, "w");
        if (f == NULL) {
            fprintf(stderr, "Server: cannot write %s\n", addr_file);
            goto cleanup_ctx;
        }
        fprintf(f, "%s", addr_str);
        fclose(f);
    }

    /* Step 1: Post unexpected recv */
    printf("Server: posting unexpected recv...\n");
    op_id = NA_Op_create(state.na_class, NA_OP_SINGLE);
    ret = NA_Msg_recv_unexpected(state.na_class, state.context,
        unexpected_recv_cb, &state, state.recv_buf, sizeof(state.recv_buf),
        NULL, op_id);
    if (ret != NA_SUCCESS) {
        fprintf(stderr, "Server: NA_Msg_recv_unexpected failed: %d\n", ret);
        goto cleanup_op;
    }

    ret = progress_loop(&state, &state.unexpected_done);
    if (ret != NA_SUCCESS)
        goto cleanup_op;

    printf("Server: received unexpected msg (size=%zu, tag=%u): \"%.*s\"\n",
        state.recv_size, state.recv_tag,
        (int) state.recv_size, state.recv_buf);

    /* Verify content */
    if (strncmp(state.recv_buf, "Hello from client", 17) != 0) {
        fprintf(stderr, "Server: unexpected message mismatch!\n");
        goto cleanup_op;
    }

    NA_Op_destroy(state.na_class, op_id);

    /* Step 2: Post expected recv from client */
    printf("Server: posting expected recv...\n");
    op_id = NA_Op_create(state.na_class, NA_OP_SINGLE);
    ret = NA_Msg_recv_expected(state.na_class, state.context,
        expected_recv_cb, &state, state.expected_buf,
        sizeof(state.expected_buf), NULL,
        state.client_addr, 0, 200, op_id);
    if (ret != NA_SUCCESS) {
        fprintf(stderr, "Server: NA_Msg_recv_expected failed: %d\n", ret);
        goto cleanup_op;
    }

    ret = progress_loop(&state, &state.expected_done);
    if (ret != NA_SUCCESS)
        goto cleanup_op;

    printf("Server: received expected msg (size=%zu): \"%.*s\"\n",
        state.expected_size, (int) state.expected_size, state.expected_buf);

    NA_Op_destroy(state.na_class, op_id);

    /* Step 3: Send expected reply */
    printf("Server: sending expected reply...\n");
    {
        const char *reply = "Reply from server";
        op_id = NA_Op_create(state.na_class, NA_OP_SINGLE);
        ret = NA_Msg_send_expected(state.na_class, state.context,
            send_cb, &state, reply, strlen(reply), NULL,
            state.client_addr, 0, 300, op_id);
        if (ret != NA_SUCCESS) {
            fprintf(stderr, "Server: NA_Msg_send_expected failed: %d\n", ret);
            goto cleanup_op;
        }

        ret = progress_loop(&state, &state.send_done);
        if (ret != NA_SUCCESS)
            goto cleanup_op;
    }

    printf("Server: expected reply sent\n");
    rc = EXIT_SUCCESS;
    printf("Server: === PASS ===\n");

cleanup_op:
    NA_Op_destroy(state.na_class, op_id);
    if (state.client_addr)
        NA_Addr_free(state.na_class, state.client_addr);
cleanup_self:
    if (self_addr)
        NA_Addr_free(state.na_class, self_addr);
cleanup_ctx:
    NA_Context_destroy(state.na_class, state.context);
cleanup_class:
    NA_Finalize(state.na_class);
done:
    /* Clean up address file */
    unlink(addr_file);
    return rc;
}
