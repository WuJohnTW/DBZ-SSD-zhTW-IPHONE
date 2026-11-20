// SDL3 supplies the UIKit application delegate and native process entry on iOS.
// Keep the actual game entry in Rust; this Objective-C translation unit is the bridge.
#include <SDL3/SDL_main.h>

extern int ssd_ios_main(int argc, char **argv);

int main(int argc, char **argv)
{
    return ssd_ios_main(argc, argv);
}
