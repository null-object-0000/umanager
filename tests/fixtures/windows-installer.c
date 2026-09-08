// Harmless Wine integration fixture: installs only copies of itself inside the
// disposable WINEPREFIX. No network, user documents, or system modifications.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <string.h>
#ifndef PATCH_VERSION
#define PATCH_VERSION 1
#endif
// Fixed-file version payload used by the installed-version reader.
volatile const DWORD test_version[] = {0xfeef04bd, 0x10000, 5 << 16, PATCH_VERSION << 16, 0, 0};
int main(void) {
    char self[MAX_PATH];
    if (test_version[2] != (5 << 16)) return 9;
    GetModuleFileNameA(NULL, self, MAX_PATH);
    if (strstr(self, "Uninstall.exe")) {
        return DeleteFileA("C:\\Program Files (x86)\\WXWork\\WXWork.exe") ? 0 : 2;
    }
    CreateDirectoryA("C:\\Program Files (x86)\\WXWork", NULL);
    if (!CopyFileA(self, "C:\\Program Files (x86)\\WXWork\\WXWork.exe", FALSE)) return 3;
    if (!CopyFileA(self, "C:\\Program Files (x86)\\WXWork\\Uninstall.exe", FALSE)) return 4;
    return 0;
}
