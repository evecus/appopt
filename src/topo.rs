// CPU 拓扑自动识别
// 通过 /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq 识别核心等级
// 天玑9400+ (MT6991): 4xA720@2400MHz(Little) + 3xX4@3300MHz(Big) + 1xX925@3730MHz(Prime)
// 通用策略：按最大频率自动分组，频率最高的为Prime，次高为Big，其余为Little

use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone)]
pub struct CpuTopology {
    /// 超大核：频率最高的1个或少数几个（Prime）
    pub prime: Vec<u32>,
    /// 大核：频率次高组（Big）
    pub big: Vec<u32>,
    /// 小核/能效核：频率最低组（Little）
    pub little: Vec<u32>,
    /// 全部核心
    pub all: Vec<u32>,
    /// 核心总数
    pub count: u32,
}

impl CpuTopology {
    /// 从 /sys 读取 CPU 拓扑，自动识别 Prime/Big/Little
    pub fn detect() -> Self {
        let mut freq_map: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut all = Vec::new();

        // 读取每个核心的最大频率
        for i in 0..16u32 {
            let path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/cpuinfo_max_freq", i);
            match fs::read_to_string(&path) {
                Ok(content) => {
                    let freq: u32 = content.trim().parse().unwrap_or(0);
                    if freq > 0 {
                        freq_map.entry(freq).or_default().push(i);
                        all.push(i);
                    }
                }
                Err(_) => break,
            }
        }

        all.sort();
        let count = all.len() as u32;

        if freq_map.is_empty() {
            // fallback：无法读取频率，全部当 big 用
            return CpuTopology {
                prime: vec![],
                big: all.clone(),
                little: vec![],
                all,
                count,
            };
        }

        // 按频率从高到低排序
        let mut freq_groups: Vec<(u32, Vec<u32>)> = freq_map.into_iter().collect();
        freq_groups.sort_by(|a, b| b.0.cmp(&a.0));

        // 分组策略：
        // - 最高频率组 = Prime（通常1个，但有些SoC是2个）
        // - 次高频率组 = Big
        // - 其余 = Little
        //
        // 特殊处理：如果最高频率组只有1个核心，且次高频率组存在，则按 1/n-1/rest 分
        // 如果只有2个频率段（如骁龙8系一些型号），最高段全当Big，无Prime单独划分

        let (prime, big, little) = match freq_groups.len() {
            1 => {
                // 所有核心同频，全当 big
                (vec![], freq_groups[0].1.clone(), vec![])
            }
            2 => {
                // 两段频率：高段=Big，低段=Little，无Prime
                // 适合 4+4 这类架构
                (vec![], freq_groups[0].1.clone(), freq_groups[1].1.clone())
            }
            _ => {
                // 三段及以上：最高=Prime，次高=Big，其余=Little
                let prime = freq_groups[0].1.clone();
                let big = freq_groups[1].1.clone();
                let little: Vec<u32> = freq_groups[2..]
                    .iter()
                    .flat_map(|(_, cores)| cores.iter().cloned())
                    .collect();
                (prime, big, little)
            }
        };

        eprintln!(
            "[topo] 检测到 {} 核心: Prime={:?} Big={:?} Little={:?}",
            count, prime, big, little
        );

        CpuTopology { prime, big, little, all, count }
    }

    /// 将核心列表转为 cpuset 字符串，如 [4,5,6,7] -> "4-7"
    pub fn cores_to_cpuset(cores: &[u32]) -> String {
        if cores.is_empty() {
            return String::new();
        }
        let mut sorted = cores.to_vec();
        sorted.sort();
        sorted.dedup();

        let mut ranges: Vec<String> = Vec::new();
        let mut start = sorted[0];
        let mut end = sorted[0];

        for &c in &sorted[1..] {
            if c == end + 1 {
                end = c;
            } else {
                if start == end {
                    ranges.push(format!("{}", start));
                } else {
                    ranges.push(format!("{}-{}", start, end));
                }
                start = c;
                end = c;
            }
        }
        if start == end {
            ranges.push(format!("{}", start));
        } else {
            ranges.push(format!("{}-{}", start, end));
        }
        ranges.join(",")
    }

    /// 获取 Prime 的 cpuset 字符串，Prime 不存在时 fallback 到 Big 最后一个核
    pub fn prime_cpuset(&self) -> String {
        if !self.prime.is_empty() {
            Self::cores_to_cpuset(&self.prime)
        } else if !self.big.is_empty() {
            // fallback：取 big 里频率最高的（已排序，取最后一个）
            Self::cores_to_cpuset(&[*self.big.last().unwrap()])
        } else {
            Self::cores_to_cpuset(&self.all)
        }
    }

    pub fn big_cpuset(&self) -> String {
        if !self.big.is_empty() {
            Self::cores_to_cpuset(&self.big)
        } else {
            Self::cores_to_cpuset(&self.all)
        }
    }

    pub fn big_and_prime_cpuset(&self) -> String {
        let mut cores = self.big.clone();
        cores.extend_from_slice(&self.prime);
        Self::cores_to_cpuset(&cores)
    }

    pub fn little_cpuset(&self) -> String {
        if !self.little.is_empty() {
            Self::cores_to_cpuset(&self.little)
        } else {
            Self::cores_to_cpuset(&self.all)
        }
    }

    pub fn all_cpuset(&self) -> String {
        Self::cores_to_cpuset(&self.all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cores_to_cpuset() {
        assert_eq!(CpuTopology::cores_to_cpuset(&[0,1,2,3]), "0-3");
        assert_eq!(CpuTopology::cores_to_cpuset(&[4,5,6]), "4-6");
        assert_eq!(CpuTopology::cores_to_cpuset(&[7]), "7");
        assert_eq!(CpuTopology::cores_to_cpuset(&[0,1,4,5,7]), "0-1,4-5,7");
    }
}
