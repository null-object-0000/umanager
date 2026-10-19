/*
 * 中文字体回归探针（Wine）：对若干界面字体取 "中文" 的字形索引。
 *
 * 判据：GetGlyphIndicesW 用 GGI_MARK_NONEXISTING_GLYPHS 时，0xFFFF 表示该字符没有
 * 字形，也就是界面上的「方框」。
 *
 * 用法：wine font-probe.exe <zh|link>
 *   zh   —— 中文区域（LC_ALL=zh_CN.UTF-8）：GB2312 字符集下所有界面字体都必须画出中文
 *          （中文应用给界面字体设的就是中文代码页字符集）；另外 Windows 中文界面常用的
 *          几个字族在 DEFAULT_CHARSET 下也必须能画出中文。
 *   link —— 英文区域（LC_ALL=C.UTF-8）：只有宿主上不存在的字族需要被 HKLM 的
 *          FontLink\SystemLink 救回来，**这一组才是对「FontLink 写没写对」敏感的断言**。
 *
 * 为什么 link 模式是 FontLink 的判据（wine-11.20 + fonts-noto-cjk，一次性 prefix 在
 * zh_CN 下 wineboot 初始化，然后分别跑探针实测）：
 *
 *   环境                                    Microsoft YaHei / SimSun / NSimSun / YaHei UI
 *   ------------------------------------------------------------------------------------
 *   zh_CN + 无 SystemLink                   Noto Sans CJK SC，有字形（fontconfig 回退）
 *   C.UTF-8 + HKLM 有 SystemLink            Noto Sans CJK SC，有字形
 *   C.UTF-8 + 整个 SystemLink 键被删除      Arial，0xFFFF（方框）
 *
 * 也就是说：中文区域下即使完全没有 SystemLink 也能画出中文，用 zh 模式无法证明 FontLink
 * 生效；只有英文区域下依赖 SystemLink 的那几个字族才能把「写对 HKLM」与「写错 HKCU /
 * 没写」区分开。反方向的坑也测过：给宿主**存在**的字族（Tahoma / Arial / MS Sans Serif）
 * 写 SystemLink 会顶掉 fontconfig 回退，把 GB2312 场景从「AR PL UKai CN 有字形」变成
 * 0xFFFF，所以 zh 模式同时是这类反效果的回归。
 *
 * 输出：每个组合一行 "<name> cs=<charset> resolved=<face> g0=.. g1=.. missing=YES/NO
 * expect=.."；全部符合期望时退出码 0。
 */
#include <windows.h>
#include <stdio.h>
#include <string.h>

typedef struct
{
    const WCHAR *face;
    BYTE charset;
    int must_have_cjk;
} expectation;

/* zh：GB2312 下所有界面字体都不能出现方框。 */
static const expectation zh_expectations[] = {
    { L"Tahoma",               GB2312_CHARSET,  1 },
    { L"Arial",                GB2312_CHARSET,  1 },
    { L"MS Sans Serif",        GB2312_CHARSET,  1 },
    { L"MS Shell Dlg",         GB2312_CHARSET,  1 },
    { L"Microsoft Sans Serif", GB2312_CHARSET,  1 },
    { L"Microsoft YaHei",      GB2312_CHARSET,  1 },
    { L"Microsoft YaHei UI",   GB2312_CHARSET,  1 },
    { L"SimSun",               GB2312_CHARSET,  1 },
    { L"NSimSun",              GB2312_CHARSET,  1 },
    /* DEFAULT：Windows 中文界面常用的字族同样要有中文。 */
    { L"Microsoft Sans Serif", DEFAULT_CHARSET, 1 },
    { L"Microsoft YaHei",      DEFAULT_CHARSET, 1 },
    { L"Microsoft YaHei UI",   DEFAULT_CHARSET, 1 },
    { L"SimSun",               DEFAULT_CHARSET, 1 },
    { L"NSimSun",              DEFAULT_CHARSET, 1 },
};

/* link：英文区域下，只有宿主上不存在的字族会被 SystemLink 救回来。 */
static const expectation link_expectations[] = {
    { L"Microsoft YaHei",    DEFAULT_CHARSET, 1 },
    { L"Microsoft YaHei UI", DEFAULT_CHARSET, 1 },
    { L"SimSun",             DEFAULT_CHARSET, 1 },
    { L"NSimSun",            DEFAULT_CHARSET, 1 },
};

static int check(const expectation *e)
{
    /* "中文" 用码点写，避免源码编码影响测试。 */
    static const WCHAR text[2] = { 0x4e2d, 0x6587 };
    WORD glyphs[2] = { 0, 0 };
    WCHAR resolved[LF_FACESIZE] = { 0 };
    LOGFONTW lf = { 0 };
    HDC dc;
    HFONT font;
    DWORD count;
    int missing;

    lf.lfHeight = -20;
    lf.lfCharSet = e->charset;
    lstrcpynW(lf.lfFaceName, e->face, LF_FACESIZE);

    dc = CreateCompatibleDC(NULL);
    font = CreateFontIndirectW(&lf);
    SelectObject(dc, font);
    GetTextFaceW(dc, LF_FACESIZE, resolved);
    count = GetGlyphIndicesW(dc, text, 2, glyphs, GGI_MARK_NONEXISTING_GLYPHS);
    missing = count != 2 || glyphs[0] == 0xffff || glyphs[1] == 0xffff;
    wprintf(L"%-20s cs=%-4d resolved=%-22s g0=%04x g1=%04x missing=%s expect=%s\n",
            e->face, e->charset, resolved, glyphs[0], glyphs[1],
            missing ? L"YES" : L"NO", e->must_have_cjk ? L"CJK" : L"ANY");
    fflush(stdout);

    DeleteObject(font);
    DeleteDC(dc);
    return e->must_have_cjk ? !missing : 1;
}

static int run(const expectation *table, int count)
{
    int ok = 1, i;

    for (i = 0; i < count; i++)
        ok &= check(&table[i]);
    return ok;
}

int main(int argc, char **argv)
{
    const char *mode = argc > 1 ? argv[1] : "zh";

    if (!strcmp(mode, "link"))
        return run(link_expectations,
                   (int)(sizeof(link_expectations) / sizeof(link_expectations[0])))
                   ? 0
                   : 1;
    if (!strcmp(mode, "zh"))
        return run(zh_expectations, (int)(sizeof(zh_expectations) / sizeof(zh_expectations[0])))
                   ? 0
                   : 1;
    printf("usage: font-probe <zh|link>\n");
    return 2;
}
