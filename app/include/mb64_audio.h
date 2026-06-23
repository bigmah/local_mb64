// Audio output for the Mario Builder 64 macOS port. See mb64_audio.cpp.
#pragma once

#include <cstddef>
#include <cstdint>

namespace mb64::audio {
    // Open the SDL audio device at the given host output frequency.
    void reset(uint32_t output_freq);
    // ultramodern audio callbacks (called from the game's audio thread).
    void queue_samples(int16_t* audio_data, size_t sample_count);
    size_t get_frames_remaining();
    void set_frequency(uint32_t freq);
}
