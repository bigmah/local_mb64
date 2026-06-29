// Minimal entry point for the Mario Builder 64 macOS port.
//
// This drives librecomp's recomp::start(), which owns RDRAM, ROM loading, the
// game thread, and the libultra event loop. We provide: the GameEntry, the
// mandatory callbacks (get_rsp_microcode + create_render_context), the SDL/Metal
// window, and minimal stubs for the rest. Audio/input start as stubs (silent,
// no input) so we can reach a first frame before wiring them up properly.
//
// NOTE: create_render_context is provided by the RT64 render integration
// (mb64_render.cpp, adapted from the recomp template) — the next piece to land.

#include <cstdio>
#include <cstdlib>
#include <cinttypes>
#include <memory>
#include <mutex>

#define SDL_MAIN_HANDLED
#include "SDL2/SDL.h"
#include "SDL2/SDL_syswm.h"

#include "ultramodern/ultramodern.hpp"
#include "ultramodern/renderer_context.hpp"
#include "ultramodern/config.hpp"
#include "ultramodern/error_handling.hpp"
#include "librecomp/game.hpp"
#include "librecomp/rsp.hpp"

#include "mb64_input.h"
#include "mb64_audio.h"

// ── provided by the RT64 render integration (mb64_render.cpp) ──────────────────
namespace mb64::renderer {
    std::unique_ptr<ultramodern::renderer::RendererContext>
    create_render_context(uint8_t* rdram, ultramodern::renderer::WindowHandle window_handle, bool developer_mode);
}

// ── provided by mb64_overlays.cpp ─────────────────────────────────────────────
namespace mb64 {
    void register_overlays();
    void register_resident_function_addresses(uint8_t* rdram, recomp_context* ctx);
}

// The game id registered below; also used by the deferred start.
static constexpr const char8_t* kGameId = u8"mb64.us";

// ── deferred game start (called by the renderer) ──────────────────────────────
// The renderer (mb64_render.cpp) calls this once it has presented its first few
// dummy VI frames. By then ultramodern's VI thread has run set_dummy_vi() and
// initialized the VI mode, so it's safe to flip the game to "started". Starting
// any earlier makes vi_thread_func() dereference a null OSViMode and crash,
// because set_dummy_vi() only runs while the game has NOT started yet.
namespace mb64 {
    void start_game_deferred() {
        static std::once_flag once;
        std::call_once(once, [] { recomp::start_game(kGameId); });
    }
}

// ── the recompiled game entry (N64Recomp output) ──────────────────────────────
extern "C" void recomp_entrypoint(uint8_t* rdram, recomp_context* ctx);

static const recomp::Version project_version{ 0, 1, 0, "" };
static SDL_Window* g_window = nullptr;

// ── gfx callbacks ─────────────────────────────────────────────────────────────
static ultramodern::gfx_callbacks_t::gfx_data_t create_gfx() {
    SDL_SetHint(SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS, "1");
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMECONTROLLER | SDL_INIT_AUDIO) != 0) {
        fprintf(stderr, "Failed to init SDL2: %s\n", SDL_GetError());
        ULTRAMODERN_QUICK_EXIT();
    }
    fprintf(stdout, "SDL video driver: %s\n", SDL_GetCurrentVideoDriver());
    // Open the host audio device (48 kHz). The game sets its own input rate via
    // set_frequency once audio init runs.
    mb64::audio::reset(48000);
    return {};
}

// The game ships no settings UI; the launcher passes window options through the
// environment. Absent or malformed values fall back to the historical defaults,
// so running the binary by hand is unchanged.
static int env_window_dim(const char* name, int fallback) {
    const char* v = getenv(name);
    if (v == nullptr || v[0] == '\0') return fallback;
    char* end = nullptr;
    long parsed = strtol(v, &end, 10);
    if (end == v || parsed < 320 || parsed > 16384) return fallback;
    return (int)parsed;
}
static bool env_fullscreen() {
    const char* v = getenv("MB64_FULLSCREEN");
    return v != nullptr && (v[0] == '1' || v[0] == 't' || v[0] == 'T' || v[0] == 'y' || v[0] == 'Y');
}

// Resolution upscaling factor the launcher requests via MB64_RES_SCALE:
//   0  → match the window (RT64 renders at an integer multiple of native 240p
//        that fills the output — sharp at any window size)
//   N  → a fixed multiple of native 240p (1 = native 240p, 2 = 480p, 4 = 960p, …)
// Absent/invalid falls back to native 1x, so running the binary by hand keeps the
// historical look. Capped at RT64's resolutionMultiplier limit (32).
static int env_res_scale() {
    const char* v = getenv("MB64_RES_SCALE");
    if (v == nullptr || v[0] == '\0') return 1;
    char* end = nullptr;
    long parsed = strtol(v, &end, 10);
    if (end == v || parsed < 0 || parsed > 32) return 1;
    return (int)parsed;
}

static ultramodern::renderer::WindowHandle create_window(ultramodern::gfx_callbacks_t::gfx_data_t) {
    const int width = env_window_dim("MB64_WINDOW_WIDTH", 1600);
    const int height = env_window_dim("MB64_WINDOW_HEIGHT", 960);
    uint32_t flags = SDL_WINDOW_RESIZABLE | SDL_WINDOW_METAL;
    if (env_fullscreen()) {
        flags |= SDL_WINDOW_FULLSCREEN_DESKTOP;
    }
    g_window = SDL_CreateWindow("Mario Builder 64", SDL_WINDOWPOS_CENTERED, SDL_WINDOWPOS_CENTERED,
                                width, height, flags);
    if (g_window == nullptr) {
        fprintf(stderr, "Failed to create window: %s\n", SDL_GetError());
        ULTRAMODERN_QUICK_EXIT();
    }
    SDL_SysWMinfo wmInfo;
    SDL_VERSION(&wmInfo.version);
    SDL_GetWindowWMInfo(g_window, &wmInfo);
    SDL_MetalView view = SDL_Metal_CreateView(g_window);
    return ultramodern::renderer::WindowHandle{ wmInfo.info.cocoa.window, SDL_Metal_GetLayer(view) };
}

static void update_gfx(void*) {
    // Pump SDL events on the gfx thread; quit on window close; feed input hotplug.
    SDL_Event ev;
    while (SDL_PollEvent(&ev)) {
        if (ev.type == SDL_QUIT) {
            // Exit immediately rather than unwinding through RT64's Metal-thread
            // teardown, which has an autorelease-pool / objc_release lifetime race
            // that segfaults on shutdown (the "RT64 Workload" thread faults in
            // objc_autoreleasePoolPop at _pthread_exit). The virtual SD card flushes
            // after every write (mb64_sdcard.cpp), so a hard exit loses no level
            // data; flush stdio for good measure. quick_exit() == std::_Exit() on
            // macOS: no dtors, no thread joins, OS reclaims everything.
            fflush(nullptr);
            ultramodern::error_handling::quick_exit(__FILE__, __LINE__, __func__, EXIT_SUCCESS);
        }
        mb64::input::handle_event(ev);
    }
    // Snapshot keyboard + controller into the N64 port-0 state for get_n64_input.
    mb64::input::update();
}

// ── rsp microcode (audio HLE'd later; gfx is intercepted by RT64) ─────────────
// A no-op RSP audio microcode: completes the task without synthesizing samples.
// The audio system MUST run (level loading touches the sound banks and hangs if
// they're uninitialized), but real audio synthesis — recompiling aspMain with
// RSPRecomp — is milestone M3a. Returning a ucode that just reports `Broke`
// (normal completion) keeps audio tasks non-fatal and yields silence for now;
// returning nullptr would make run_task abort the whole app.
static RspExitReason mb64_null_audio_ucode(uint8_t* /*rdram*/, uint32_t /*ucode_addr*/) {
    return RspExitReason::Broke;
}
// The recompiled SM64/SDK audio microcode (rsp/aspMain.cpp, via RSPRecomp).
// C++ linkage, matching the generated definition (cf. template `extern RspUcodeFunc aspMain;`).
RspExitReason aspMain(uint8_t* rdram, uint32_t ucode_addr);
static RspUcodeFunc* get_rsp_microcode(const OSTask* task) {
    // Gfx tasks (M_GFXTASK) never reach here — RT64 intercepts them. Audio tasks
    // run the recompiled aspMain to synthesize samples; anything else gets the
    // harmless no-op ucode (returning nullptr would abort the app).
    if (task->t.type == M_AUDTASK) {
        return aspMain;
    }
    return mb64_null_audio_ucode;
}

// ── input (keyboard + SDL game controller → N64 port 0; see mb64_input.cpp) ────
// poll_input runs on the game/controller thread; the actual SDL polling happens
// on the gfx thread in update_gfx (mb64::input::update), which snapshots state
// into atomics that get_n64_input reads — so poll_input itself is a no-op.
static void poll_input() {}
static bool get_n64_input(int port, uint16_t* buttons, float* x, float* y) {
    return mb64::input::get(port, buttons, x, y);
}
static void set_rumble(int, bool) {}
static ultramodern::input::connected_device_info_t get_connected_device_info(int port) {
    return mb64::input::device_info(port);
}

// ── audio (recompiled aspMain → SDL; see mb64_audio.cpp) ──────────────────────
static void queue_samples(int16_t* data, size_t count) { mb64::audio::queue_samples(data, count); }
static size_t get_frames_remaining() { return mb64::audio::get_frames_remaining(); }
static void set_frequency(uint32_t freq) { mb64::audio::set_frequency(freq); }

// ── misc stubs ────────────────────────────────────────────────────────────────
static void vi_callback() {}
static void gfx_init_callback() {}
static void message_box(const char* msg) { fprintf(stderr, "[message_box] %s\n", msg); }
static std::string get_game_thread_name(const OSThread*) { return "mb64_game"; }

// The entrypoint is a KSEG0 address and must be stored sign-extended into the
// 64-bit gpr, matching the recompiler's address convention (RDRAM accesses via
// MEM_* subtract the sign-extended 0xFFFFFFFF80000000). Returning the bare
// 0x80124330u would zero-extend and make the boot ROM->RDRAM copy in init()
// write 0x80000000 bytes past RDRAM (segfault in do_rom_read).
gpr get_entrypoint_address() { return (gpr)(int32_t)0x80124330; }

static std::vector<recomp::GameEntry> supported_games = {
    {
        .rom_hash = 0xd82b295c5a4d30f5ULL, // XXH3_64bits of the (4-byte-padded) US mb64.z64
        .internal_name = "MARIO BUILDER 64    ",
        .game_id = u8"mb64.us",
        .mod_game_id = "mb64",
        .save_type = recomp::SaveType::AllowAll, // permissive; refine to Sram/Flashram once confirmed
        .is_enabled = true,
        .has_compressed_code = false,
        .entrypoint_address = get_entrypoint_address(),
        .entrypoint = recomp_entrypoint,
        // Runs right after librecomp's init() (which mis-registers SM64's
        // non-contiguous resident sections) and before the entrypoint; fixes the
        // function lookup table so indirect calls into the engine resolve.
        .on_init_callback = mb64::register_resident_function_addresses,
    },
};

int main(int, char**) {
    SDL_SetMainReady();

    recomp::register_config_path(std::filesystem::current_path());
    for (const auto& game : supported_games) {
        recomp::register_game(game);
    }
    mb64::register_overlays();

    // ── provision the ROM ─────────────────────────────────────────────────────
    // librecomp normally relies on the (RmlUi) launcher menu to provision a ROM
    // and call start_game(). This port has no in-engine UI, so we provision the
    // ROM into the config path here (on first run). The actual start_game() is
    // deferred to start_game_deferred() once the renderer is up (see above) —
    // provisioning alone does NOT start the game, so it's safe to do eagerly.
    std::u8string game_id{ kGameId };
    recomp::check_all_stored_roms();
    if (!recomp::is_rom_valid(game_id)) {
        const std::filesystem::path source_rom = "mb64.z64";
        recomp::RomValidationError rom_err = recomp::select_rom(source_rom, game_id);
        if (rom_err != recomp::RomValidationError::Good) {
            fprintf(stderr,
                "Failed to provision ROM from '%s' (RomValidationError %d). Place a "
                "valid US Mario Builder 64 ROM named 'mb64.z64' next to the executable.\n",
                source_rom.string().c_str(), (int)rom_err);
            return 1;
        }
        fprintf(stdout, "Provisioned Mario Builder 64 ROM into the config path.\n");
    }

    // Seed the graphics config the RT64 integration reads at startup (window mode
    // in mb64_main's create_window expectations; resolution + downsampling in
    // set_application_user_config). The game ships no in-engine settings UI, so
    // this in-memory config is the single source of truth — nothing is persisted.
    // All fields a fresh GraphicsConfig{} doesn't touch stay at the historical
    // default, so an unconfigured launch (no env vars) is unchanged.
    {
        ultramodern::renderer::GraphicsConfig gfx_cfg{};
        gfx_cfg.wm_option = env_fullscreen()
            ? ultramodern::renderer::WindowMode::Fullscreen
            : ultramodern::renderer::WindowMode::Windowed;
        const int res_scale = env_res_scale();
        if (res_scale <= 0) {
            // Match the window (integer-scaled to fill it).
            gfx_cfg.res_option = ultramodern::renderer::Resolution::Auto;
        } else {
            // Fixed multiple of native 240p.
            gfx_cfg.res_option = ultramodern::renderer::Resolution::Original;
            gfx_cfg.ds_option = res_scale;
        }
        ultramodern::renderer::set_graphics_config(gfx_cfg);
    }

    recomp::rsp::callbacks_t rsp_callbacks{ .get_rsp_microcode = get_rsp_microcode };
    ultramodern::renderer::callbacks_t renderer_callbacks{ .create_render_context = mb64::renderer::create_render_context };
    ultramodern::audio_callbacks_t audio_callbacks{ .queue_samples = queue_samples, .get_frames_remaining = get_frames_remaining, .set_frequency = set_frequency };
    ultramodern::input::callbacks_t input_callbacks{ .poll_input = poll_input, .get_input = get_n64_input, .set_rumble = set_rumble, .get_connected_device_info = get_connected_device_info };
    ultramodern::gfx_callbacks_t gfx_callbacks{ .create_gfx = create_gfx, .create_window = create_window, .update_gfx = update_gfx };
    ultramodern::events::callbacks_t events_callbacks{ .vi_callback = vi_callback, .gfx_init_callback = gfx_init_callback };
    ultramodern::error_handling::callbacks_t error_callbacks{ .message_box = message_box };
    ultramodern::threads::callbacks_t threads_callbacks{ .get_game_thread_name = get_game_thread_name };

    recomp::start(
        project_version,
        {},
        rsp_callbacks,
        renderer_callbacks,
        audio_callbacks,
        input_callbacks,
        gfx_callbacks,
        events_callbacks,
        error_callbacks,
        threads_callbacks
    );

    return 0;
}
