/* UManager 中国节假日日历扩展
 *
 * 在 GNOME 顶部日历面板中为中国大陆法定节假日与调休上班日做标记：
 *  - 格子里第一行显示公历日期，第二行显示节日名 + 休/班（如「中秋休」「春节班」）
 *  - 休=红色、班=绿色
 *
 * 数据来自扩展目录内的 holidays.json（由 UManager 写入/刷新）。
 * 每次日历菜单打开时重新读取数据文件，因此 UManager 更新数据后无需
 * 重启 GNOME Shell，重新打开日历即生效。
 *
 * 说明：GNOME 45+ 移除了 St.Widget.set_tooltip_text()（那是 Gtk 组件的 API），
 * 因此节日名直接写进日期按钮的 label。顶部日历格子固定 3em×3em、容量有限，
 * 无法容纳手机日历那样的「右上角彩色徽标 + 农历 + 节气」布局，故采用
 * 「日期 + 节日名/休班」两行文字的简洁呈现。
 */

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const REST_CLASS = 'umanager-holiday-rest';
const WORK_CLASS = 'umanager-holiday-work';

/** 监视这些文件，变化时自动无注销重载（GNOME Shell 登录时才扫描扩展目录）。 */
const HOT_FILES = ['extension.js', 'stylesheet.css', 'metadata.json', 'holidays.json'];

/** 节日显示名。普通 2-3 字节日名原样保留（3em 格子放得下完整「国庆节」「中秋节」）：
 *  仅「国庆节、中秋节」这类合并的 7 字名需要压缩为「国庆中秋」。 */
const SHORT_NAMES = {
    '国庆节、中秋节': '国庆中秋',
};

/** 取节日显示名：拆掉「、」合并项，合并项用 SHORT_NAMES 压缩，普通名原样保留。 */
function shortName(name) {
    const parts = String(name ?? '').split(/[、,，]/);
    const joined = parts.map(part => SHORT_NAMES[part] ?? part).join('');
    return joined.length > 4 ? joined.slice(0, 4) : joined;
}

export default class UManagerCalendarExtension extends Extension {
    enable() {
        // 版本探针：日志里若看到 V3，说明加载的是最新版。
        console.log(`[UManager日历] enable V3`);
        this._holidays = new Map(); // 'YYYY-MM-DD' -> { name, isOffDay }
        this._originalRebuild = null;
        this._openStateId = 0;
        this._calendar = null;
        this._lastCodeStamp = 0;

        const dateMenu = Main.panel?.statusArea?.dateMenu;
        if (!dateMenu || !dateMenu._calendar)
            return;

        this._calendar = dateMenu._calendar;

        // Hook 日历重建：切换月份会重建日期按钮网格，重建后应用标记。
        const calendar = this._calendar;
        this._originalRebuild = calendar._rebuildCalendar.bind(calendar);
        calendar._rebuildCalendar = () => {
            this._originalRebuild();
            this._applyMarks();
        };

        // 每次打开日历时：先做一次"代码版本检查"（若源码/样式更新过则自动重载，避免循环），
        // 再重读数据并应用标记。代码更新（如 UManager 发新版扩展）无需注销即可生效。
        this._openStateId = dateMenu.menu.connect('open-state-changed', (menu, open) => {
            if (!open)
                return;
            // 用 setTimeout 把重载放到 open 事件之后，避免与本次绘制互相干扰。
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 50, () => {
                if (this._codeUpdatedSinceLoad()) {
                    this._reloadExtension();
                    return GLib.SOURCE_REMOVE;
                }
                this._refresh();
                return GLib.SOURCE_REMOVE;
            });
        });

        this._lastCodeStamp = this._codeStamp();
        this._refresh();
    }

    disable() {
        if (this._calendar && this._originalRebuild)
            this._calendar._rebuildCalendar = this._originalRebuild;
        if (this._calendar && this._openStateId) {
            const dateMenu = Main.panel?.statusArea?.dateMenu;
            if (dateMenu)
                dateMenu.menu.disconnect(this._openStateId);
        }
        this._clearMarks();
        this._calendar = null;
        this._holidays = null;
    }

    /** 扩展代码相关文件（js/css/metadata）的最新修改时间戳；变化则说明需要重载。 */
    _codeStamp() {
        let latest = 0;
        for (const name of HOT_FILES) {
            if (name === 'holidays.json')
                continue; // 数据文件由 _refresh 内存重读，无需重载代码。
            try {
                const f = this.dir.get_child(name);
                const info = f.query_info('time::modified', Gio.FileQueryInfoFlags.NONE, null);
                const mtime = info.get_modification_time().tv_sec;
                if (mtime > latest)
                    latest = mtime;
            } catch (_e) {
                // 文件不存在则忽略。
            }
        }
        return latest;
    }

    _codeUpdatedSinceLoad() {
        return this._codeStamp() > this._lastCodeStamp;
    }

    _reloadExtension() {
        const manager = Main.extensionManager;
        if (!manager || typeof manager.reloadExtension !== 'function')
            return;
        try {
            const self = Extension.lookupByUUID(this.uuid);
            if (self)
                manager.reloadExtension(self);
        } catch (error) {
            console.warn(`UManager 日历扩展：自动重载失败 - ${error}`);
        }
    }

    _refresh() {
        this._loadData();
        this._applyMarks();
    }

    _loadData() {
        this._holidays = new Map();
        try {
            const file = this.dir.get_child('holidays.json');
            const [ok, contents] = file.load_contents(null);
            if (!ok)
                return;
            const data = JSON.parse(new TextDecoder().decode(contents));
            for (const day of data.days ?? []) {
                if (day && day.date && day.name)
                    this._holidays.set(day.date, { name: day.name, isOffDay: !!day.isOffDay });
            }
        } catch (error) {
            console.warn(`UManager 日历扩展：读取 holidays.json 失败 - ${error}`);
        }
    }

    _dateKey(date) {
        const y = date.getFullYear();
        const m = String(date.getMonth() + 1).padStart(2, '0');
        const d = String(date.getDate()).padStart(2, '0');
        return `${y}-${m}-${d}`;
    }

    /** 该日期是否是一个假期段的首日：若前一天也是节假日/调休日，则为段中/末尾。 */
    _isHolidayStart(date) {
        const prev = new Date(date);
        prev.setDate(prev.getDate() - 1);
        return !this._holidays.has(this._dateKey(prev));
    }

    _applyMarks() {
        this._clearMarks();
        if (!this._calendar || !this._holidays)
            return;

        for (const button of this._calendar._buttons ?? []) {
            if (!button._date)
                continue;
            const holiday = this._holidays.get(this._dateKey(button._date));
            if (!holiday)
                continue;

            const day = String(button._date.getDate());
            // 调休上班日：只标「班」，不写因为什么节日要上班。
            // 节假日首日：直接显示节日名（如「中秋」「国庆节」），不加「休」。
            // 节假日段内其他天：只标「休」。
            let label;
            if (!holiday.isOffDay) {
                label = '班';
            } else if (this._isHolidayStart(button._date)) {
                label = shortName(holiday.name);
            } else {
                label = '休';
            }
            const styleClass = holiday.isOffDay ? REST_CLASS : WORK_CLASS;
            button.add_style_class_name(styleClass);
            button.set_label(`${day}\n${label}`);
        }
    }

    _clearMarks() {
        if (!this._calendar)
            return;
        for (const button of this._calendar._buttons ?? []) {
            if (!button)
                continue;
            button.remove_style_class_name(REST_CLASS);
            button.remove_style_class_name(WORK_CLASS);
            if (button._date)
                button.set_label(String(button._date.getDate()));
        }
    }
}
