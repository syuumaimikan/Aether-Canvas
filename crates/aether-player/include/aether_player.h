/*
 * aether_player.h — C API for the Aether Canvas runtime.
 *
 * Link against the aether_player shared library (build it with
 * `cargo build --release -p aether-player`). The same functions are the
 * exports of the WebAssembly module used by runtime/web/aether-player.js.
 *
 * Conventions
 *   - A player is an opaque handle from aether_player_new(), released with
 *     aether_player_free(). Every function accepts NULL and then does
 *     nothing and returns 0, -1 or NULL.
 *   - Strings are UTF-8, returned as a pointer plus a length written
 *     through `len` (which may be NULL). They are NUL-terminated too.
 *     They stay valid until the player is freed; event names only until the
 *     next tick.
 *   - Positions are in document pixels (the canvas the model was painted
 *     on), y down. Texture coordinates are 0..1, v down (row 0 is the top
 *     of the PNG).
 *   - Colour is straight (non-premultiplied) in the PNGs. Premultiply on
 *     upload and blend premultiplied:
 *       normal   ONE, ONE_MINUS_SRC_ALPHA
 *       multiply DST_COLOR, ONE_MINUS_SRC_ALPHA
 *       screen   ONE, ONE_MINUS_SRC_COLOR
 *       add      ONE, ONE
 *   - Tint: rgb = rgb * multiply; rgb = rgb + screen - rgb * screen (on
 *     straight colour), then alpha *= opacity.
 *   - A draw item with mask >= 0 is drawn only where the mask part covers:
 *     render the mask part's alpha (times mask_opacity) and multiply by it.
 *
 * A typical frame:
 *
 *   aether_player_tick(p, dt);
 *   const AetherDrawItem *items = aether_player_draw_items(p);
 *   for (uint32_t i = 0; i < aether_player_draw_count(p); i++) {
 *       const AetherDrawItem *d = &items[i];
 *       const float *xy = aether_player_part_positions(p, d->part);
 *       // draw part d->part: xy + uvs + indices, texture page
 *       // aether_player_part_texture(p, d->part), with d's blend and tint
 *   }
 */
#ifndef AETHER_PLAYER_H
#define AETHER_PLAYER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AETHER_API_VERSION 1

typedef struct AetherPlayer AetherPlayer;

typedef enum AetherBlend {
    AETHER_BLEND_NORMAL = 0,
    AETHER_BLEND_MULTIPLY = 1,
    AETHER_BLEND_SCREEN = 2,
    AETHER_BLEND_ADD = 3
} AetherBlend;

typedef enum AetherStage {
    AETHER_STAGE_MOTIONS = 0,
    AETHER_STAGE_BEHAVIOURS = 1,
    AETHER_STAGE_DRIVERS = 2,
    AETHER_STAGE_PHYSICS = 3,
    AETHER_STAGE_JIGGLE = 4
} AetherStage;

/* One draw call. Layout matches aether_player::DrawItem (44 bytes). */
typedef struct AetherDrawItem {
    uint32_t part;        /* part to draw */
    int32_t mask;         /* part whose coverage clips this one, or -1 */
    uint32_t blend;       /* AetherBlend */
    float opacity;        /* final opacity */
    float mask_opacity;   /* opacity for drawing the mask part's coverage */
    float multiply[3];    /* multiply tint */
    float screen[3];      /* screen tint */
} AetherDrawItem;

/* Memory and errors */
uint32_t aether_api_version(void);
uint8_t *aether_alloc(size_t len);
void aether_dealloc(uint8_t *ptr, size_t len);
const uint8_t *aether_last_error(size_t *len);

/* Lifetime */
AetherPlayer *aether_player_new(const uint8_t *json, size_t len);
void aether_player_free(AetherPlayer *p);

/* Canvas and textures */
uint32_t aether_player_width(const AetherPlayer *p);
uint32_t aether_player_height(const AetherPlayer *p);
uint32_t aether_player_texture_count(const AetherPlayer *p);
const uint8_t *aether_player_texture_file(const AetherPlayer *p, uint32_t index, size_t *len);

/* Parameters */
uint32_t aether_player_parameter_count(const AetherPlayer *p);
const uint8_t *aether_player_parameter_name(const AetherPlayer *p, uint32_t index, size_t *len);
int32_t aether_player_parameter_find(const AetherPlayer *p, const uint8_t *name, size_t len);
/* Writes min, max, default into out[0..3]. Returns 0 for a bad index. */
uint32_t aether_player_parameter_range(const AetherPlayer *p, uint32_t index, float *out);
float aether_player_parameter_value(const AetherPlayer *p, uint32_t index);
void aether_player_set_parameter(AetherPlayer *p, uint32_t index, float value);

/* Motions, expressions and inputs */
uint32_t aether_player_motion_count(const AetherPlayer *p);
const uint8_t *aether_player_motion_name(const AetherPlayer *p, uint32_t index, size_t *len);
float aether_player_motion_duration(const AetherPlayer *p, uint32_t index);
uint32_t aether_player_play_motion(AetherPlayer *p, uint32_t index, uint32_t additive);
void aether_player_stop_motions(AetherPlayer *p);
uint32_t aether_player_is_playing(const AetherPlayer *p);
uint32_t aether_player_expression_count(const AetherPlayer *p);
const uint8_t *aether_player_expression_name(const AetherPlayer *p, uint32_t index, size_t *len);
void aether_player_set_expression(AetherPlayer *p, int32_t index); /* < 0: none */
void aether_player_look_at(AetherPlayer *p, float x, float y);   /* -1..1, y up */
void aether_player_look_ahead(AetherPlayer *p);
void aether_player_set_audio(AetherPlayer *p, float level, float brightness);
void aether_player_set_stage(AetherPlayer *p, uint32_t stage, uint32_t enabled);

/* Time */
void aether_player_tick(AetherPlayer *p, float dt);
void aether_player_update(AetherPlayer *p);
void aether_player_reset(AetherPlayer *p);
uint32_t aether_player_event_count(const AetherPlayer *p);
const uint8_t *aether_player_event_name(const AetherPlayer *p, uint32_t index, size_t *len);
int32_t aether_player_event_motion(const AetherPlayer *p, uint32_t index);

/* Geometry and drawing */
uint32_t aether_player_part_count(const AetherPlayer *p);
const uint8_t *aether_player_part_name(const AetherPlayer *p, uint32_t index, size_t *len);
uint32_t aether_player_part_texture(const AetherPlayer *p, uint32_t index);
uint32_t aether_player_part_vertex_count(const AetherPlayer *p, uint32_t index);
const float *aether_player_part_uvs(const AetherPlayer *p, uint32_t index);
uint32_t aether_player_part_index_count(const AetherPlayer *p, uint32_t index);
const uint32_t *aether_player_part_indices(const AetherPlayer *p, uint32_t index);
const float *aether_player_part_positions(const AetherPlayer *p, uint32_t index);
uint32_t aether_player_draw_count(const AetherPlayer *p);
const AetherDrawItem *aether_player_draw_items(const AetherPlayer *p);
int32_t aether_player_hit_test(const AetherPlayer *p, float x, float y);

#ifdef __cplusplus
}
#endif

#endif /* AETHER_PLAYER_H */
