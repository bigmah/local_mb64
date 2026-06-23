#ifndef __MB64_RENDER_H__
#define __MB64_RENDER_H__

#include <memory>

#include "common/rt64_user_configuration.h"
#include "ultramodern/renderer_context.hpp"

namespace RT64 {
    struct Application;
}

namespace mb64 {
    namespace renderer {
        // RT64-backed render context for Mario Builder 64. Adapted from the
        // recomp template's zelda64::renderer::RT64Context, decoupled from the
        // RmlUi UI and the mods/texture-pack system (which this port does not use).
        class RT64Context final : public ultramodern::renderer::RendererContext {
        public:
            ~RT64Context() override;
            RT64Context(uint8_t *rdram, ultramodern::renderer::WindowHandle window_handle, bool developer_mode);

            bool valid() override { return static_cast<bool>(app); }

            bool update_config(const ultramodern::renderer::GraphicsConfig &old_config, const ultramodern::renderer::GraphicsConfig &new_config) override;

            void enable_instant_present() override;
            void send_dl(const OSTask *task) override;
            void update_screen() override;
            void shutdown() override;
            uint32_t get_display_framerate() const override;
            float get_resolution_scale() const override;

        private:
            std::unique_ptr<RT64::Application> app;
        };

        std::unique_ptr<ultramodern::renderer::RendererContext> create_render_context(uint8_t *rdram, ultramodern::renderer::WindowHandle window_handle, bool developer_mode);

        RT64::UserConfiguration::Antialiasing RT64MaxMSAA();
        bool RT64SamplePositionsSupported();
        bool RT64HighPrecisionFBEnabled();
    }
}

#endif
