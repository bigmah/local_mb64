// Input for the Mario Builder 64 macOS port: SDL keyboard + game controller
// mapped to an N64 controller (port 0). Self-contained — does NOT use the
// drmario/Zelda template's RmlUi-based config system; remapping will live in the
// Rust+Dioxus launcher later.
//
// Threading: SDL events are pumped on the gfx thread (update_gfx → SDL_PollEvent).
// We snapshot the derived N64 state there (mb64::input::update) into atomics, and
// the game/controller thread reads that snapshot (mb64::input::get). poll_input
// stays a no-op.

#include <atomic>
#include <cstdint>

#include "SDL2/SDL.h"

#include "ultramodern/input.hpp"

#include "mb64_input.h"

namespace {
    // N64 OSContPad.button bits (libultra CONT_*; see vendor os_cont.h).
    constexpr uint16_t N_A      = 0x8000;
    constexpr uint16_t N_B      = 0x4000;
    constexpr uint16_t N_Z      = 0x2000;
    constexpr uint16_t N_START  = 0x1000;
    constexpr uint16_t N_DUP    = 0x0800;
    constexpr uint16_t N_DDOWN  = 0x0400;
    constexpr uint16_t N_DLEFT  = 0x0200;
    constexpr uint16_t N_DRIGHT = 0x0100;
    constexpr uint16_t N_L      = 0x0020;
    constexpr uint16_t N_R      = 0x0010;
    constexpr uint16_t N_CUP    = 0x0008;
    constexpr uint16_t N_CDOWN  = 0x0004;
    constexpr uint16_t N_CLEFT  = 0x0002;
    constexpr uint16_t N_CRIGHT = 0x0001;

    std::atomic<uint16_t> g_buttons{ 0 };
    std::atomic<float>    g_stick_x{ 0.0f };
    std::atomic<float>    g_stick_y{ 0.0f };

    // The first opened game controller drives port 0 (alongside the keyboard).
    SDL_GameController* g_pad = nullptr;
    SDL_JoystickID      g_pad_id = -1;

    constexpr int   TRIGGER_THRESHOLD = 8000;   // 0..32767
    constexpr float STICK_DEADZONE    = 0.18f;

    float apply_axis(int16_t raw) {
        float v = raw / 32767.0f;
        if (v > 1.0f) v = 1.0f;
        if (v < -1.0f) v = -1.0f;
        return v;
    }
}

void mb64::input::handle_event(const SDL_Event& ev) {
    switch (ev.type) {
        case SDL_CONTROLLERDEVICEADDED:
            if (g_pad == nullptr && SDL_IsGameController(ev.cdevice.which)) {
                g_pad = SDL_GameControllerOpen(ev.cdevice.which);
                if (g_pad != nullptr) {
                    SDL_Joystick* js = SDL_GameControllerGetJoystick(g_pad);
                    g_pad_id = SDL_JoystickInstanceID(js);
                    fprintf(stdout, "[input] controller connected: %s\n",
                            SDL_GameControllerName(g_pad));
                }
            }
            break;
        case SDL_CONTROLLERDEVICEREMOVED:
            if (ev.cdevice.which == g_pad_id) {
                if (g_pad != nullptr) SDL_GameControllerClose(g_pad);
                g_pad = nullptr;
                g_pad_id = -1;
                fprintf(stdout, "[input] controller disconnected\n");
            }
            break;
        default:
            break;
    }
}

void mb64::input::update() {
    uint16_t buttons = 0;
    float sx = 0.0f, sy = 0.0f;

    // ── Keyboard (always active) ──────────────────────────────────────────────
    // Stick: arrow keys.  A:X  B:C  Z:Z/LShift  L:A  R:S  Start:Enter
    // C-buttons: I/J/K/L (up/left/down/right).  D-pad: T/F/G/H (up/left/down/right)
    const Uint8* ks = SDL_GetKeyboardState(nullptr);
    if (ks != nullptr) {
        if (ks[SDL_SCANCODE_LEFT])  sx -= 1.0f;
        if (ks[SDL_SCANCODE_RIGHT]) sx += 1.0f;
        if (ks[SDL_SCANCODE_UP])    sy += 1.0f;
        if (ks[SDL_SCANCODE_DOWN])  sy -= 1.0f;

        if (ks[SDL_SCANCODE_X]) buttons |= N_A;
        if (ks[SDL_SCANCODE_C]) buttons |= N_B;
        if (ks[SDL_SCANCODE_Z] || ks[SDL_SCANCODE_LSHIFT] || ks[SDL_SCANCODE_RSHIFT]) buttons |= N_Z;
        if (ks[SDL_SCANCODE_A]) buttons |= N_L;
        if (ks[SDL_SCANCODE_S]) buttons |= N_R;
        if (ks[SDL_SCANCODE_RETURN] || ks[SDL_SCANCODE_KP_ENTER]) buttons |= N_START;

        if (ks[SDL_SCANCODE_I]) buttons |= N_CUP;
        if (ks[SDL_SCANCODE_K]) buttons |= N_CDOWN;
        if (ks[SDL_SCANCODE_J]) buttons |= N_CLEFT;
        if (ks[SDL_SCANCODE_L]) buttons |= N_CRIGHT;

        if (ks[SDL_SCANCODE_T]) buttons |= N_DUP;
        if (ks[SDL_SCANCODE_G]) buttons |= N_DDOWN;
        if (ks[SDL_SCANCODE_F]) buttons |= N_DLEFT;
        if (ks[SDL_SCANCODE_H]) buttons |= N_DRIGHT;
    }

    // ── Game controller (if connected) ────────────────────────────────────────
    if (g_pad != nullptr) {
        auto down = [&](SDL_GameControllerButton b) {
            return SDL_GameControllerGetButton(g_pad, b) != 0;
        };
        if (down(SDL_CONTROLLER_BUTTON_A))             buttons |= N_A;
        if (down(SDL_CONTROLLER_BUTTON_X))             buttons |= N_B;
        if (down(SDL_CONTROLLER_BUTTON_START))         buttons |= N_START;
        if (down(SDL_CONTROLLER_BUTTON_LEFTSHOULDER))  buttons |= N_L;
        if (down(SDL_CONTROLLER_BUTTON_RIGHTSHOULDER)) buttons |= N_R;
        if (down(SDL_CONTROLLER_BUTTON_DPAD_UP))       buttons |= N_DUP;
        if (down(SDL_CONTROLLER_BUTTON_DPAD_DOWN))     buttons |= N_DDOWN;
        if (down(SDL_CONTROLLER_BUTTON_DPAD_LEFT))     buttons |= N_DLEFT;
        if (down(SDL_CONTROLLER_BUTTON_DPAD_RIGHT))    buttons |= N_DRIGHT;

        // Z trigger: either trigger pressed (also B-button / right-stick-click as Z).
        if (SDL_GameControllerGetAxis(g_pad, SDL_CONTROLLER_AXIS_TRIGGERLEFT)  > TRIGGER_THRESHOLD ||
            SDL_GameControllerGetAxis(g_pad, SDL_CONTROLLER_AXIS_TRIGGERRIGHT) > TRIGGER_THRESHOLD ||
            down(SDL_CONTROLLER_BUTTON_RIGHTSTICK)) {
            buttons |= N_Z;
        }

        // C-buttons from the right stick (camera).
        int16_t rx = SDL_GameControllerGetAxis(g_pad, SDL_CONTROLLER_AXIS_RIGHTX);
        int16_t ry = SDL_GameControllerGetAxis(g_pad, SDL_CONTROLLER_AXIS_RIGHTY);
        const int16_t C_THRESH = 16000;
        if (rx < -C_THRESH) buttons |= N_CLEFT;
        if (rx >  C_THRESH) buttons |= N_CRIGHT;
        if (ry < -C_THRESH) buttons |= N_CUP;
        if (ry >  C_THRESH) buttons |= N_CDOWN;

        // Left stick → analog (overrides keyboard stick when deflected).
        float px = apply_axis(SDL_GameControllerGetAxis(g_pad, SDL_CONTROLLER_AXIS_LEFTX));
        float py = -apply_axis(SDL_GameControllerGetAxis(g_pad, SDL_CONTROLLER_AXIS_LEFTY)); // SDL down = +, N64 up = +
        float mag = px * px + py * py;
        if (mag > STICK_DEADZONE * STICK_DEADZONE) {
            sx = px;
            sy = py;
        }
    }

    // Clamp keyboard diagonal to the unit circle so it isn't faster diagonally.
    float m2 = sx * sx + sy * sy;
    if (m2 > 1.0f) {
        float inv = 1.0f / SDL_sqrtf(m2);
        sx *= inv;
        sy *= inv;
    }

    g_buttons.store(buttons, std::memory_order_relaxed);
    g_stick_x.store(sx, std::memory_order_relaxed);
    g_stick_y.store(sy, std::memory_order_relaxed);
}

bool mb64::input::get(int controller_num, uint16_t* buttons, float* x, float* y) {
    if (controller_num != 0) {
        return false; // only port 0 is connected
    }
    if (buttons) *buttons = g_buttons.load(std::memory_order_relaxed);
    if (x) *x = g_stick_x.load(std::memory_order_relaxed);
    if (y) *y = g_stick_y.load(std::memory_order_relaxed);
    return true;
}

ultramodern::input::connected_device_info_t mb64::input::device_info(int controller_num) {
    if (controller_num == 0) {
        return { ultramodern::input::Device::Controller, ultramodern::input::Pak::None };
    }
    return { ultramodern::input::Device::None, ultramodern::input::Pak::None };
}
