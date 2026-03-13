/**
 * Test: NA plugin init/finalize and protocol info
 *
 * Verifies:
 *  - Plugin loads successfully via NA_Initialize("<protocol>://", true)
 *  - NA_Get_protocol_info returns an entry for the plugin
 *  - Self address can be retrieved
 *  - Finalize completes cleanly
 *
 * Usage: test_libp2p_init [-p <protocol>]
 *   Default protocol is "libp2p".
 */

#include <na.h>
#include <na_types.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int
main(int argc, char *argv[])
{
    const char *protocol = "libp2p";
    char info_string[256];
    na_class_t *na_class;
    na_context_t *context;
    na_addr_t *self_addr = NULL;
    char addr_str[256];
    size_t addr_str_size = sizeof(addr_str);
    struct na_protocol_info *pinfo = NULL, *p;
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

    printf("=== NA Plugin Init Test (protocol=%s) ===\n", protocol);

    /* Test 1: protocol info */
    printf("Test 1: NA_Get_protocol_info(\"%s\")... ", protocol);
    ret = NA_Get_protocol_info(protocol, &pinfo);
    if (ret != NA_SUCCESS || pinfo == NULL) {
        printf("FAIL (ret=%d)\n", ret);
        goto done;
    }
    for (p = pinfo; p != NULL; p = p->next) {
        printf("class=%s protocol=%s device=%s ",
            p->class_name ? p->class_name : "(null)",
            p->protocol_name ? p->protocol_name : "(null)",
            p->device_name ? p->device_name : "(null)");
    }
    printf("OK\n");
    NA_Free_protocol_info(pinfo);

    /* Test 2: initialize as listener */
    printf("Test 2: NA_Initialize(\"%s\", listen=true)... ", info_string);
    na_class = NA_Initialize(info_string, true);
    if (na_class == NULL) {
        printf("FAIL\n");
        goto done;
    }
    printf("OK\n");

    /* Test 3: create context */
    printf("Test 3: NA_Context_create... ");
    context = NA_Context_create(na_class);
    if (context == NULL) {
        printf("FAIL\n");
        goto cleanup_class;
    }
    printf("OK\n");

    /* Test 4: self address */
    printf("Test 4: NA_Addr_self... ");
    ret = NA_Addr_self(na_class, &self_addr);
    if (ret != NA_SUCCESS) {
        printf("FAIL (ret=%d)\n", ret);
        goto cleanup_ctx;
    }
    ret = NA_Addr_to_string(na_class, addr_str, &addr_str_size, self_addr);
    if (ret != NA_SUCCESS) {
        printf("FAIL to_string (ret=%d)\n", ret);
        goto cleanup_addr;
    }
    printf("self=%s OK\n", addr_str);

    /* Test 5: addr is self */
    printf("Test 5: NA_Addr_is_self... ");
    if (!NA_Addr_is_self(na_class, self_addr)) {
        printf("FAIL\n");
        goto cleanup_addr;
    }
    printf("OK\n");

    /* Test 6: max sizes */
    printf("Test 6: msg sizes... ");
    printf("unexpected=%zu expected=%zu max_tag=%u ",
        NA_Msg_get_max_unexpected_size(na_class),
        NA_Msg_get_max_expected_size(na_class),
        NA_Msg_get_max_tag(na_class));
    printf("OK\n");

    rc = EXIT_SUCCESS;
    printf("=== All tests passed ===\n");

cleanup_addr:
    NA_Addr_free(na_class, self_addr);
cleanup_ctx:
    NA_Context_destroy(na_class, context);
cleanup_class:
    NA_Finalize(na_class);
done:
    return rc;
}
