/*
 * denia-wake: force every Denial output back to full power via
 * zwlr-output-power-management-unstable-v1. Useful when idle DPMS or a
 * stuck output left the display blank and the session is otherwise alive.
 *
 * Build: cc denia-wake.c $(pkg-config --cflags --libs wayland-client) \
 *            wlr-output-power-management-unstable-v1-client-protocol.c \
 *            -o denia-wake
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <wayland-client.h>
#include "wlr-output-power-management-unstable-v1-client-protocol.h"

struct output_state {
    struct wl_output *output;
    uint32_t name;
    struct output_state *next;
};

static struct zwlr_output_power_manager_v1 *power_manager = NULL;
static struct output_state *outputs = NULL;

static void registry_global(void *data, struct wl_registry *registry,
                            uint32_t name, const char *interface,
                            uint32_t version) {
    if (strcmp(interface, zwlr_output_power_manager_v1_interface.name) == 0) {
        power_manager = wl_registry_bind(
            registry, name, &zwlr_output_power_manager_v1_interface, 1);
    } else if (strcmp(interface, wl_output_interface.name) == 0) {
        struct output_state *state = calloc(1, sizeof(*state));
        if (state == NULL) {
            fprintf(stderr, "denia-wake: out of memory\n");
            exit(1);
        }
        state->output = wl_registry_bind(registry, name, &wl_output_interface, 1);
        state->name = name;
        state->next = outputs;
        outputs = state;
    }
}

static void registry_global_remove(void *data, struct wl_registry *registry,
                                   uint32_t name) {
    /* Keep the list; the compositor is expected to outlive this tool. */
}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_global_remove,
};

int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    if (display == NULL) {
        fprintf(stderr, "denia-wake: cannot connect to WAYLAND_DISPLAY=%s\n",
                getenv("WAYLAND_DISPLAY"));
        return 1;
    }

    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);

    if (power_manager == NULL) {
        fprintf(stderr,
                "denia-wake: compositor does not advertise "
                "zwlr_output_power_manager_v1\n");
        return 1;
    }

    int changed = 0;
    for (struct output_state *state = outputs; state != NULL;
         state = state->next) {
        struct zwlr_output_power_v1 *power =
            zwlr_output_power_manager_v1_get_output_power(power_manager,
                                                          state->output);
        zwlr_output_power_v1_set_mode(power,
                                      ZWLR_OUTPUT_POWER_V1_MODE_ON);
        zwlr_output_power_v1_destroy(power);
        changed++;
    }
    wl_display_roundtrip(display);

    if (changed == 0) {
        fprintf(stderr, "denia-wake: no outputs found\n");
        return 1;
    }
    printf("denia-wake: requested power-on for %d output(s)\n", changed);
    wl_display_disconnect(display);
    return 0;
}
