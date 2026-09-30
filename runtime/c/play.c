/*
 * play.c — load a runtime model through the C API, play it, and print what
 * a renderer would draw.
 *
 *   cargo build --release -p aether-player
 *   cc runtime/c/play.c -I crates/aether-player/include \
 *      -L target/release -laether_player -o play
 *   LD_LIBRARY_PATH=target/release ./play path/to/model.json
 */
#include "aether_player.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint8_t *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *bytes = malloc(size > 0 ? (size_t)size : 1);
    *len = bytes ? fread(bytes, 1, (size_t)size, f) : 0;
    fclose(f);
    return bytes;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s model.json\n", argv[0]);
        return 2;
    }
    size_t len = 0;
    uint8_t *json = read_file(argv[1], &len);
    if (!json) {
        perror(argv[1]);
        return 1;
    }
    AetherPlayer *p = aether_player_new(json, len);
    free(json);
    if (!p) {
        size_t n = 0;
        const uint8_t *message = aether_last_error(&n);
        fprintf(stderr, "could not load: %.*s\n", (int)n, (const char *)message);
        return 1;
    }

    printf("api %u, canvas %ux%u, %u parts, %u parameters, %u motions\n", aether_api_version(),
           aether_player_width(p), aether_player_height(p), aether_player_part_count(p),
           aether_player_parameter_count(p), aether_player_motion_count(p));

    /* Turn the head by hand, then let the first motion take over. */
    const char *name = "AngleX";
    int32_t angle = aether_player_parameter_find(p, (const uint8_t *)name, strlen(name));
    if (angle >= 0) {
        float range[3];
        aether_player_parameter_range(p, (uint32_t)angle, range);
        aether_player_set_parameter(p, (uint32_t)angle, range[1]);
        aether_player_update(p);
        printf("%s set to %.1f, drawn at %.1f\n", name, range[1],
               aether_player_parameter_value(p, (uint32_t)angle));
    }
    /* Hotkeys: list them, then press "1" as a keyboard handler would. */
    for (uint32_t h = 0; h < aether_player_hotkey_count(p); h++) {
        size_t nk = 0, nn = 0;
        const uint8_t *keys = aether_player_hotkey_keys(p, h, &nk);
        const uint8_t *label = aether_player_hotkey_name(p, h, &nn);
        printf("hotkey %.*s: %.*s\n", (int)nk, (const char *)keys, (int)nn, (const char *)label);
    }
    int32_t fired = aether_player_press_key(p, (const uint8_t *)"1", 1, 0);
    if (fired >= 0) {
        printf("pressed 1: hotkey %d\n", fired);
    } else if (aether_player_motion_count(p) > 0) {
        aether_player_play_motion(p, 0, 0);
    }
    for (int frame = 0; frame < 60; frame++) {
        aether_player_tick(p, 1.0f / 60.0f);
        for (uint32_t e = 0; e < aether_player_event_count(p); e++) {
            size_t n = 0;
            const uint8_t *event = aether_player_event_name(p, e, &n);
            printf("event %.*s\n", (int)n, (const char *)event);
        }
    }

    /* What a renderer draws this frame, back to front. */
    const AetherDrawItem *items = aether_player_draw_items(p);
    uint32_t count = aether_player_draw_count(p);
    uint32_t triangles = 0;
    for (uint32_t i = 0; i < count; i++) {
        const AetherDrawItem *d = &items[i];
        const float *xy = aether_player_part_positions(p, d->part);
        size_t n = 0;
        const char *part = (const char *)aether_player_part_name(p, d->part, &n);
        triangles += aether_player_part_index_count(p, d->part) / 3;
        printf("draw %-14s blend %u opacity %.2f mask %2d first vertex (%.1f, %.1f)\n", part, d->blend,
               d->opacity, d->mask, xy[0], xy[1]);
    }
    printf("%u draw calls, %u triangles\n", count, triangles);

    float cx = aether_player_width(p) / 2.0f;
    float cy = aether_player_height(p) / 2.0f;
    int32_t hit = aether_player_hit_test(p, cx, cy);
    size_t n = 0;
    printf("hit at centre: %s\n", hit >= 0 ? (const char *)aether_player_part_name(p, (uint32_t)hit, &n) : "nothing");

    aether_player_free(p);
    return 0;
}
