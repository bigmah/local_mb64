# Minimal CMake for the Mario Builder 64 macOS port.
# Phase M1b-foundation: build the runtime stack (RT64 + ultramodern + librecomp)
# and our recompiled game code as linkable libraries. The app executable (main +
# RT64 render integration) is layered on top once this foundation is proven.
cmake_minimum_required(VERSION 3.20)

if (APPLE)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "11.0" CACHE STRING "Minimum macOS version")
endif()

project(mario_builder_64 LANGUAGES C CXX)
set(CMAKE_C_STANDARD 17)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if (APPLE)
    enable_language(OBJC OBJCXX)
endif()

set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR})
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR})

# RT64 renderer (native Metal on macOS) — proven to build standalone already.
set(RT64_STATIC TRUE)
set(RT64_SDL_WINDOW_VULKAN TRUE)
add_compile_definitions(HLSL_CPU)
add_subdirectory(${CMAKE_SOURCE_DIR}/lib/rt64 ${CMAKE_BINARY_DIR}/rt64)
target_include_directories(rt64 PRIVATE ${CMAKE_BINARY_DIR}/rt64/src)
# macOS/libc++: hlslpp's HLSL_CPU scalar path uses labs/abs without including a
# stdlib header, which libc++ doesn't pull in transitively → force-include
# <stdlib.h> (works for both rt64's C and C++ sources).
if (APPLE)
    target_compile_options(rt64 PRIVATE -include stdlib.h)
endif()

# N64ModernRuntime = ultramodern (libultra reimpl) + librecomp (recomp runtime).
add_subdirectory(${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime)

# RecompiledFuncs — our 470k lines of recompiled Mario Builder 64 (N64Recomp output).
add_library(RecompiledFuncs STATIC)
target_compile_options(RecompiledFuncs PRIVATE
    -fno-strict-aliasing
    -Wno-everything
)
target_include_directories(RecompiledFuncs PRIVATE
    ${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime/ultramodern/include
    ${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime/librecomp/include
    ${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime/N64Recomp/include
)
file(GLOB FUNC_C_SOURCES ${CMAKE_SOURCE_DIR}/RecompiledFuncs/*.c)
file(GLOB FUNC_CXX_SOURCES ${CMAKE_SOURCE_DIR}/RecompiledFuncs/*.cpp)
target_sources(RecompiledFuncs PRIVATE ${FUNC_C_SOURCES} ${FUNC_CXX_SOURCES})

# ── App executable (main + overlays + temporary render stub) ──────────────────
# Links the foundation libs to surface remaining undefined symbols on the way to
# a runnable binary. The render stub returns null until RT64Context is ported.
find_package(SDL2 REQUIRED)
find_package(Threads REQUIRED)

add_executable(mario_builder_64
    ${CMAKE_SOURCE_DIR}/src/mb64_main.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_overlays.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_render.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_input.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_si.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_sdcard.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_audio.cpp
    ${CMAKE_SOURCE_DIR}/src/mb64_ultra_stubs.cpp
    ${CMAKE_SOURCE_DIR}/rsp/aspMain.cpp
)
target_compile_options(mario_builder_64 PRIVATE -fno-strict-aliasing -Wno-everything)
target_include_directories(mario_builder_64 PRIVATE
    ${CMAKE_SOURCE_DIR}/include
    ${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime/ultramodern/include
    ${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime/librecomp/include
    ${CMAKE_SOURCE_DIR}/lib/N64ModernRuntime/N64Recomp/include
    ${CMAKE_SOURCE_DIR}/lib/rt64/src
    ${CMAKE_SOURCE_DIR}/lib/rt64/src/hle
    ${CMAKE_SOURCE_DIR}/lib/rt64/src/render
    ${CMAKE_SOURCE_DIR}/lib/rt64/src/rhi
    ${CMAKE_SOURCE_DIR}/lib/rt64/src/contrib
    ${CMAKE_SOURCE_DIR}/lib/rt64/src/contrib/hlslpp/include
    ${SDL2_INCLUDE_DIRS}
)
target_link_libraries(mario_builder_64 PRIVATE
    RecompiledFuncs
    librecomp
    ultramodern
    rt64
    nfd
    SDL2::SDL2
    Threads::Threads
)

