#define WIN32_LEAN_AND_MEAN
#include <windows.h>

// Adapted from the locally used WeCom titlebar fix. No machine-specific state.
static HWND main_window;
static HWND titlebar_window;

static BOOL belongs_to_main(HWND window)
{
    int depth;
    for (depth = 0; window && depth < 16; ++depth) {
        if (window == main_window || window == titlebar_window) return TRUE;
        window = GetWindow(window, GW_OWNER);
    }
    return 0;
}

static BOOL CALLBACK find_windows(HWND window, LPARAM unused)
{
    char class_name[128] = {0};
    (void)unused;
    if (!GetClassNameA(window, class_name, sizeof(class_name))) return TRUE;
    if (!lstrcmpiA(class_name, "WeWorkWindow")) main_window = window;
    return TRUE;
}

static BOOL CALLBACK find_titlebar(HWND window, LPARAM unused)
{
    char class_name[128] = {0};
    (void)unused;
    if (GetWindow(window, GW_OWNER) != main_window) return TRUE;
    if (!GetClassNameA(window, class_name, sizeof(class_name))) return TRUE;
    if (!lstrcmpiA(class_name, "TitleBarWindow")) titlebar_window = window;
    return TRUE;
}

int main(int argc, char **argv)
{
    HANDLE mutex = CreateMutexA(NULL, TRUE, "Local\\UManagerWeComTitlebar");
    if (!mutex || GetLastError() == ERROR_ALREADY_EXISTS) return 0;
    int startup_cycles = 0;
    int seen_main = 0;
    int absent_cycles = 0;
    int inactive_cycles = 0;
    int once = argc > 1 && argv[1][0] == 'o';

    for (;;) {
        main_window = NULL;
        titlebar_window = NULL;
        EnumWindows(find_windows, 0);
        if (main_window) {
            seen_main = 1;
            absent_cycles = 0;
            EnumWindows(find_titlebar, 0);
            if (titlebar_window) {
                BOOL active = belongs_to_main(GetForegroundWindow());
                if (active) inactive_cycles = 0;
                else if (inactive_cycles < 10) ++inactive_cycles;

                if ((IsIconic(main_window) || !IsWindowVisible(main_window) ||
                     inactive_cycles >= 2) && IsWindowVisible(titlebar_window))
                    ShowWindowAsync(titlebar_window, SW_HIDE);
                else if (active && !IsIconic(main_window) &&
                         IsWindowVisible(main_window) &&
                         !IsWindowVisible(titlebar_window))
                    ShowWindowAsync(titlebar_window, SW_SHOWNA);
            }
        } else if (seen_main && ++absent_cycles >= 30) {
            return 0;
        }
        if (!seen_main && ++startup_cycles > 600) return 0;
        if (once) return main_window ? 0 : 1;
        Sleep(100);
    }
}
