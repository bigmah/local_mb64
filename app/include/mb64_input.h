// Input integration for the Mario Builder 64 macOS port. See mb64_input.cpp.
#pragma once

#include <cstdint>

union SDL_Event;

#include "ultramodern/input.hpp"

namespace mb64::input {
    // Called from the gfx thread for each SDL event (controller hotplug).
    void handle_event(const SDL_Event& ev);
    // Called from the gfx thread each frame after pumping events: snapshots the
    // keyboard + controller state into the N64 port-0 state read by get().
    void update();
    // Input callbacks (read the snapshot; safe from the game/controller thread).
    bool get(int controller_num, uint16_t* buttons, float* x, float* y);
    ultramodern::input::connected_device_info_t device_info(int controller_num);
}
