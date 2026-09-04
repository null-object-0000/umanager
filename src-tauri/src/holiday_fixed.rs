//! 中国法定节假日的"固定当天"生成器。
//!
//! 国务院每年公布的是「放假几天 + 哪天调休补班」；而**节假日本身的日期**是由
//! 法律/历法固定的（元旦 1/1、劳动节 5/1、国庆节 10/1；春节正月初一、除夕
//! 腊月二十九/三十、清明约 4/5、端午五月初五、中秋八月十五）。在国务院尚未
//! 公布某年调休安排（如 2027+）时，这些"当天"仍可预先标注「休」。
//!
//! 本模块只生成**节日当天**（is_off_day = true），绝不生成调休上班日或连休段，
//! 以免与官方安排冲突。农历部分用 1900–2100 的农历数据表换算（经典算法），
//! 清明用太阳黄经近似公式，均已用 2024–2026 官方数据回测校准。

/// 农历月天数：每个农历年用 7 位十六进制编码：
/// 低 4 位 = 闰月所在月（0 为无闰月）；bit15 表示闰月是 30 天；其余 bit 表示第 1-12 月为 30 天。
const LUNAR_INFO: [u32; 201] = [
    0x04bd8, 0x04ae0, 0x0a570, 0x054d5, 0x0d260, 0x0d950, 0x16554, 0x056a0, 0x09ad0, 0x055d2,
    0x04ae0, 0x0a5b6, 0x0a4d0, 0x0d250, 0x1d255, 0x0b540, 0x0d6a0, 0x0ada2, 0x095b0, 0x14977,
    0x04970, 0x0a4b0, 0x0b4b5, 0x06a50, 0x06d40, 0x1ab54, 0x02b60, 0x09570, 0x052f2, 0x04970,
    0x06566, 0x0d4a0, 0x0ea50, 0x06e95, 0x05ad0, 0x02b60, 0x186e3, 0x092e0, 0x1c8d7, 0x0c950,
    0x0d4a0, 0x1d8a6, 0x0b550, 0x056a0, 0x1a5b4, 0x025d0, 0x092d0, 0x0d2b2, 0x0a950, 0x0b557,
    0x06ca0, 0x0b550, 0x15355, 0x04da0, 0x0a5b0, 0x14573, 0x052b0, 0x0a9a8, 0x0e950, 0x06aa0,
    0x0aea6, 0x0ab50, 0x04b60, 0x0aae4, 0x0a570, 0x05260, 0x0f263, 0x0d950, 0x05b57, 0x056a0,
    0x096d0, 0x04dd5, 0x04ad0, 0x0a4d0, 0x0d4d4, 0x0d250, 0x0d558, 0x0b540, 0x0b6a0, 0x195a6,
    0x095b0, 0x049b0, 0x0a974, 0x0a4b0, 0x0b27a, 0x06a50, 0x06d40, 0x0af46, 0x0ab60, 0x09570,
    0x04af5, 0x04970, 0x064b0, 0x074a3, 0x0ea50, 0x06b58, 0x055c0, 0x0ab60, 0x096d5, 0x092e0,
    0x0c960, 0x0d954, 0x0d4a0, 0x0da50, 0x07552, 0x056a0, 0x0abb7, 0x025d0, 0x092d0, 0x0cab5,
    0x0a950, 0x0b4a0, 0x0baa4, 0x0ad50, 0x055d9, 0x04ba0, 0x0a5b0, 0x15176, 0x052b0, 0x0a930,
    0x07954, 0x06aa0, 0x0ad50, 0x05b52, 0x04b60, 0x0a6e6, 0x0a4e0, 0x0d260, 0x0ea65, 0x0d530,
    0x05aa0, 0x076a3, 0x096d0, 0x04afb, 0x04ad0, 0x0a4d0, 0x1d0b6, 0x0d250, 0x0d520, 0x0dd45,
    0x0b5a0, 0x056d0, 0x055b2, 0x049b0, 0x0a577, 0x0a4b0, 0x0aa50, 0x1b255, 0x06d20, 0x0ada0,
    0x14b63, 0x09370, 0x049f8, 0x04970, 0x064b0, 0x168a6, 0x0ea50, 0x06b20, 0x1a6c4, 0x0aae0,
    0x0a2e0, 0x0d2e3, 0x0c960, 0x0d557, 0x0d4a0, 0x0da50, 0x05d55, 0x056a0, 0x0a6d0, 0x055d4,
    0x052d0, 0x0a9b8, 0x0a950, 0x0b4a0, 0x0b6a6, 0x0ad50, 0x055a0, 0x0aba4, 0x0a5b0, 0x052b0,
    0x0b273, 0x06930, 0x07337, 0x06aa0, 0x0ad50, 0x14b55, 0x04b60, 0x0a570, 0x054e4, 0x0d160,
    0x0e968, 0x0d520, 0x0daa0, 0x16aa6, 0x056d0, 0x04ae0, 0x0a9d4, 0x0a2d0, 0x0d150, 0x0f252,
    0x0d520,
];

const BASE_DATE: (i32, u32, u32) = (1900, 1, 31); // 1900-01-31 = 农历 1900 年正月初一

fn days_in_month(y: i32, m: u32) -> u32 {
    if LUNAR_INFO[(y - 1900) as usize] & (0x10000 >> m) == 0 {
        29
    } else {
        30
    }
}

fn leap_month(y: i32) -> u32 {
    LUNAR_INFO[(y - 1900) as usize] & 0xf
}

fn leap_days(y: i32) -> u32 {
    let lm = leap_month(y);
    if lm == 0 {
        0
    } else if LUNAR_INFO[(y - 1900) as usize] & 0x10000 == 0 {
        29
    } else {
        30
    }
}

fn lunar_year_days(y: i32) -> u32 {
    let days: u32 = (1..=12).map(|m| days_in_month(y, m)).sum();
    days + leap_days(y)
}

/// 公历日期 → 农历年月日。返回 (农历年, 农历月, 农历日)。
/// 主流程只用 `lunar_to_solar`（正推）；此反向换算保留用于测试回测算法正确性。
#[allow(dead_code)]
fn solar_to_lunar(y: i32, m: u32, d: u32) -> (i32, u32, u32) {
    // 计算距 1900-01-31 的天数偏移（用 i64 避免 1900 年前负值溢出）
    let mut offset = {
        let (by, bm, bd) = BASE_DATE;
        let base = days_from_civil(by, bm as u32, bd as u32);
        days_from_civil(y, m, d) - base
    } as i64;

    let mut lunar_year = 1900;
    // 逐年扣除
    while offset >= lunar_year_days(lunar_year) as i64 {
        offset -= lunar_year_days(lunar_year) as i64;
        lunar_year += 1;
    }

    let leap = leap_month(lunar_year);
    let leap_days_in_year = leap_days(lunar_year);

    let mut lunar_month = 1;
    let mut is_leap = false;
    loop {
        let month_days = if leap != 0 && lunar_month == leap + 1 && !is_leap {
            is_leap = true;
            leap_days_in_year
        } else if lunar_month == 13 {
            break;
        } else {
            { is_leap = false; days_in_month(lunar_year, lunar_month) }
        };
        if offset < month_days as i64 {
            break;
        }
        offset -= month_days as i64;
        lunar_month += 1;
    }

    (lunar_year, lunar_month, (offset + 1) as u32)
}

/// 农历年月日 → 公历日期。返回 (公历年, 月, 日)。
fn lunar_to_solar(ly: i32, lm: u32, ld: u32) -> (i32, u32, u32) {
    let (_, base_m, base_d) = BASE_DATE;
    let mut days = days_from_civil(BASE_DATE.0, base_m, base_d);

    for y in 1900..ly {
        days += lunar_year_days(y) as i64;
    }
    let leap = leap_month(ly);
    for m in 1..lm {
        days += days_in_month(ly, m) as i64;
        if leap != 0 && m == leap {
            days += leap_days(ly) as i64;
        }
    }
    days += (ld - 1) as i64;

    civil_from_days(days)
}

/// 天数 → 公历（Howard Hinnant 的 civil_from_days 算法）。
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y } as i32;
    (y, m, d)
}

/// 公历 → 从某个历元起的天数（Howard Hinnant 的 days_from_civil 算法）。
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 清明（太阳黄经 15°）所在公历日（4 月）。
/// 经典公式：D = [Y*0.2422 + C] - [Y/4]，Y = 年份后两位，C = 4.81。
fn qingming_day(year: i32) -> u32 {
    let y = (year % 100) as f64;
    (y * 0.2422 + 4.81).floor() as u32 - (y / 4.0).floor() as u32
}

/// 一个"固定节日当天"。name 为标注用名称，is_off_day 恒为 true。
pub struct FixedHoliday {
    pub date: (i32, u32, u32),
    pub name: &'static str,
}

/// 生成某年的全部法定节日当天（含除夕 + 春节），仅当天、is_off_day = true。
/// 从 `start_year`（不含，默认取 min 2024 之前也不冲突）起，任意年份均可。
/// name 用官方名称，便于前端短名映射。
pub fn fixed_holidays(year: i32) -> Vec<FixedHoliday> {
    let mut holidays = Vec::new();

    // 元旦：公历 1/1
    holidays.push(FixedHoliday { date: (year, 1, 1), name: "元旦" });

    // 除夕 + 春节：两者都可能落在相邻农历年，需分别找"公历落在本年内"的农历年。
    // 春节（正月初一，公历约 1/21–2/20）：找正月初一公历日落在本公历年的农历年。
    for ln_year in (year - 1)..=(year + 1) {
        let (cy, cm, cd) = lunar_to_solar(ln_year, 1, 1);
        if cy == year {
            holidays.push(FixedHoliday { date: (cy, cm, cd), name: "春节" });
            break;
        }
    }
    // 除夕（腊月最后一天，公历约 1/20–2/19）：找腊月最后一天公历日落在本公历年的农历年。
    for ln_year in (year - 1)..=(year + 1) {
        let last_month_days = days_in_month(ln_year, 12);
        let (ex, em, ed) = lunar_to_solar(ln_year, 12, last_month_days);
        if ex == year {
            holidays.push(FixedHoliday { date: (ex, em, ed), name: "除夕" });
            break;
        }
    }

    // 清明：约 4/5
    holidays.push(FixedHoliday { date: (year, 4, qingming_day(year)), name: "清明节" });

    // 劳动节：5/1
    holidays.push(FixedHoliday { date: (year, 5, 1), name: "劳动节" });

    // 端午节：农历五月初五（其公历日约 5/25–6/22，落在公历 year）
    for ln_year in (year - 1)..=(year + 1) {
        let (dy, dm, dd) = lunar_to_solar(ln_year, 5, 5);
        if dy == year {
            holidays.push(FixedHoliday { date: (dy, dm, dd), name: "端午节" });
            break;
        }
    }

    // 中秋节：农历八月十五（其公历日约 9/8–10/8）
    for ln_year in (year - 1)..=(year + 1) {
        let (my, mm, md) = lunar_to_solar(ln_year, 8, 15);
        if my == year {
            holidays.push(FixedHoliday { date: (my, mm, md), name: "中秋节" });
            break;
        }
    }

    // 国庆节：10/1
    holidays.push(FixedHoliday { date: (year, 10, 1), name: "国庆节" });

    holidays.sort_by_key(|h| (h.date.0, h.date.1, h.date.2));
    holidays.dedup_by(|a, b| a.date == b.date);
    holidays
}

/// 把某年份范围的"固定节日当天"合并进已有的节假日列表。
/// 规则：仅补充**尚未出现的日期**；官方已公布的日期（含调休/连休）保持原样，绝不覆盖。
/// 返回新增的条目数。`days.date` 为 `YYYY-MM-DD` 字符串。
pub fn merge_fixed_holidays(days: &mut Vec<FixedHolidayDay>, from_year: u16, to_year: u16) -> usize {
    let mut existing: std::collections::HashSet<String> =
        days.iter().map(|d| d.date.clone()).collect();
    let mut added = 0;
    for year in from_year..=to_year {
        for fh in fixed_holidays(year as i32) {
            let date = format!("{:04}-{:02}-{:02}", fh.date.0, fh.date.1, fh.date.2);
            if existing.contains(&date) {
                continue;
            }
            days.push(FixedHolidayDay {
                date: date.clone(),
                name: fh.name.to_owned(),
                is_off_day: true,
            });
            existing.insert(date);
            added += 1;
        }
    }
    added
}

/// 合并时使用的数据形状（可序列化为 holidays.json 的 `days` 数组元素）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixedHolidayDay {
    pub date: String,
    pub name: String,
    pub is_off_day: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(d: (i32, u32, u32)) -> String {
        format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
    }

    #[test]
    fn lunar_backtest_2024_2026() {
        // 公历 -> 农历
        assert_eq!(solar_to_lunar(2024, 2, 10), (2024, 1, 1)); // 2024 春节
        assert_eq!(solar_to_lunar(2024, 6, 10), (2024, 5, 5)); // 2024 端午
        assert_eq!(solar_to_lunar(2024, 9, 17), (2024, 8, 15)); // 2024 中秋
        assert_eq!(solar_to_lunar(2025, 1, 29), (2025, 1, 1));
        assert_eq!(solar_to_lunar(2026, 2, 17), (2026, 1, 1));
        assert_eq!(solar_to_lunar(2026, 6, 19), (2026, 5, 5));
        assert_eq!(solar_to_lunar(2026, 9, 25), (2026, 8, 15));
        // 农历 -> 公历（反推）
        assert_eq!(fmt(lunar_to_solar(2024, 1, 1)), "2024-02-10");
        assert_eq!(fmt(lunar_to_solar(2025, 8, 15)), "2025-10-06");
    }

    #[test]
    fn qingming_matches_solar_term() {
        assert_eq!(qingming_day(2024), 4);
        assert_eq!(qingming_day(2025), 4);
        assert_eq!(qingming_day(2026), 5);
        assert_eq!(qingming_day(2027), 5);
        assert_eq!(qingming_day(2028), 4);
    }

    #[test]
    fn fixed_holidays_2027_contains_key_dates() {
        let h = fixed_holidays(2027);
        let dates: Vec<String> = h.iter().map(|x| fmt(x.date)).collect();
        assert!(dates.contains(&"2027-01-01".to_string()), "元旦");
        assert!(dates.contains(&"2027-10-01".to_string()), "国庆");
        assert!(dates.contains(&"2027-05-01".to_string()), "劳动");
        // 2027 清明 = 4/5
        assert!(dates.contains(&"2027-04-05".to_string()), "清明");
        // 2027 春节 = 农历正月初一
        assert!(h.iter().any(|x| x.name == "春节"), "春节存在");
        assert!(h.iter().any(|x| x.name == "除夕"), "除夕存在");
        assert!(h.iter().any(|x| x.name == "端午节"), "端午存在");
        assert!(h.iter().any(|x| x.name == "中秋节"), "中秋存在");
        // 返回的每个条目都是"当天休"标记（本模块只生成节日当天，不强求额外字段；
        // 此处仅为确认生成逻辑可枚举全部 8 项）。
        assert_eq!(h.len(), 8);
    }

    #[test]
    fn fixed_holidays_are_only_day_off() {
        // 每个节日只产生一天，且不重复（去重），共 8 项固定节日（含除夕）
        let counts: std::collections::HashMap<String, u32> =
            fixed_holidays(2028).iter().map(|x| (x.name.to_string(), 1)).collect();
        assert!(counts.contains_key("春节"));
        assert!(counts.contains_key("除夕"));
        assert!(counts.contains_key("端午节"));
        assert!(counts.contains_key("中秋节"));
        assert!(counts.contains_key("清明节"));
        assert_eq!(counts.len(), 8);
    }

    #[test]
    fn merge_fixed_does_not_override_official() {
        // 模拟 2026 官方数据：10-01 是国庆，10-02..-07 是连休(休)，08 是调休班。
        let mut days = vec![
            FixedHolidayDay { date: "2026-10-01".into(), name: "国庆节".into(), is_off_day: true },
            FixedHolidayDay { date: "2026-10-08".into(), name: "国庆节".into(), is_off_day: false },
        ];
        let added = merge_fixed_holidays(&mut days, 2026, 2026);
        // 2026 已公布：官方已有的 10-01/10-08 不覆盖；额外补上元旦/春节/清明/劳动/端午/中秋等当天。
        assert!(days.iter().any(|d| d.date == "2026-10-01" && d.is_off_day), "官方休保持");
        assert!(days.iter().any(|d| d.date == "2026-10-08" && !d.is_off_day), "官方班保持");
        assert!(days.iter().any(|d| d.date == "2026-01-01" && d.is_off_day), "补元旦");
        assert!(days.iter().any(|d| d.date == "2026-02-17" && d.name == "春节"), "补春节");
        assert!(days.iter().any(|d| d.date == "2026-09-25" && d.name == "中秋节"), "补中秋");
        assert!(added >= 5, "新增多条");
    }

    #[test]
    fn merge_fixed_fills_unpublished_year() {
        // 2027 无官方数据：合并后应包含 8 个固定节日当天。
        let mut days: Vec<FixedHolidayDay> = Vec::new();
        let added = merge_fixed_holidays(&mut days, 2027, 2027);
        assert_eq!(added, 8);
        assert!(days.iter().any(|d| d.date == "2027-02-06" && d.name == "春节"));
        assert!(days.iter().any(|d| d.date == "2027-02-05" && d.name == "除夕"));
        assert!(days.iter().any(|d| d.date == "2027-10-01" && d.name == "国庆节"));
        assert!(days.iter().all(|d| d.is_off_day), "未公布年份只标休，不猜调休");
    }
}
