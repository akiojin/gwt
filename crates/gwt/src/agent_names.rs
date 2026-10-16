//! Project-scoped, localized Agent identities (Issue #3824).

use fs2::FileExt;
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::Path,
};

// Keep the catalog in the same language as the visible identity. Each catalog
// contains over 200 real given names, spread across initials, sounds and lengths.
const EN_NAMES: &str = "
Ada Alice Amelia Anna Audrey Arthur Asher Austin
Beatrice Bella Beth Blake Bonnie Brandon Brian Brooke
Caleb Camilla Cara Casey Charles Chloe Clara Cole
Daisy Daniel Daphne David Dean Delia Diana Dylan
Eden Edgar Edith Eleanor Elena Eli Emma Ethan
Faith Faye Felix Finn Fiona Flora Frances Frank
Gabriel Gail Gemma George Gideon Grace Grant Gwen
Hannah Harriet Harvey Hazel Hector Henry Holly Hugo
Ian Ida Imogen India Ingrid Irene Iris Isaac
Jack Jacob Jade James Jane Jasper Jonah Julia
Kate Keira Kelly Kelsey Kenneth Kevin Kira Kyle
Lara Laura Leo Liam Lily Logan Louis Lucy
Mabel Maeve Marcus Maria Maya Miles Molly Morgan
Nadia Naomi Nathan Nell Nicholas Nina Noah Nora
Olive Oliver Olivia Omar Opal Orla Oscar Owen
Paige Parker Patrick Paul Pearl Penny Peter Phoebe
Quentin Quinn Quincy Quintin Quinlan Quinton Queenie Quiana
Rachel Ralph Rebecca Reid Rhys Riley Rose Ruby
Sadie Sam Sarah Scott Seth Simon Sophie Stella
Tara Taylor Tessa Theo Thomas Tobias Toby Tristan
Una Ulysses Ulric Upton Ursula Uri Uriah Unity
Valerie Vanessa Vera Vernon Victor Victoria Vince Violet
Wade Walter Warren Wendy Wesley Whitney William Willow
Xander Xavier Xena Xenia Ximena Xiomara Xavion Xyla
Yale Yasmin Yara Yvette Yolanda York Yusuf Yvonne
Zachary Zara Zane Zelda Zena Zia Zoe Zora
";
const JA_NAMES: &str = "
アイ アオイ アカネ アキ アキラ アサヒ アスカ アズサ アツシ アヤ アユミ アラタ アリサ
イオリ イクミ イズミ イチカ イッセイ イツキ イブキ イヨ イロハ
ウタ ウミ ウララ ウキョウ ウメ ウノ
エイジ エイタ エツコ エナ エマ エミ エリ エリカ
オト オトハ オサム オウジ オウカ オウタ オリエ
カイ カエデ カオリ カズキ カズハ カナ カナタ カナメ カホ カリン
キイ キキ キコ キョウ キョウカ キョウヘイ キヨ キヨミ キララ
クミ クミコ クルミ クニオ クニコ クオン クララ
ケイ ケイコ ケイタ ケイゴ ケント ケン ケンジ
コウ コウタ コウキ コウヘイ ココ ココナ コト コトネ コハル コマチ
サエ サキ サクラ サチ サチコ サト サトシ サナ サヤ サラ
シオリ シズカ シノ シノブ シホ シュウ シュン ショウ ショウタ シン シンタロウ
スイ スズ スズカ スズネ スバル スミ スミレ スナオ
セイ セイジ セイナ セイラ セツ セナ セリ セリナ
ソウ ソウタ ソウマ ソノ ソノカ ソラ
タイガ タイチ タクミ タケシ タケル タツキ タツヤ タマキ
チアキ チカ チサ チトセ チナ チハル チヒロ
ツカサ ツキ ツバキ ツバサ ツムギ ツヨシ
テツ テツオ テツヤ テル テルミ テン テンマ
トウカ トウマ トキ トシオ トモ トモキ トモヤ トワ
ナオ ナオキ ナギ ナギサ ナツ ナツキ ナツミ ナナ ナナミ ナル ナルミ
ニイナ ニコ ニナ ニジ ニジカ ニシキ
ネネ ネイロ ネム ノア ノゾミ ノドカ ノノ ノリ ノリコ
ハジメ ハナ ハノン ハヤテ ハヤト ハル ハルカ ハルキ ハルト ハルナ
ヒカリ ヒカル ヒサコ ヒトミ ヒナ ヒナタ ヒビキ ヒマリ ヒロ ヒロキ
フウ フウカ フウタ フク フミ フミカ フミヤ フユ フユカ
ヘイジ ヘイタ ホノカ ホタル ホシ ホクト ホナミ ホマレ
マイ マオ マキ マコ マコト マサキ マサト マナ マナミ マホ マユ マリ
ミオ ミカ ミキ ミク ミサ ミズキ ミツキ ミドリ ミナ ミノリ ミホ ミヤビ
ムギ ムツキ ムツミ ムネオ メイ メグ メグミ メイカ
モエ モトキ モナ モミジ モモ モモカ モリオ
ヤエ ヤス ヤスコ ヤスヒロ ヤマト ヤヨイ
ユイ ユウ ユウカ ユウキ ユウタ ユウナ ユキ ユズ ユズキ ユナ ユメ ユリ
ヨウ ヨウコ ヨシ ヨシキ ヨシノ ヨリ ヨリコ
ライ ライト ライム ラン ランカ ララ
リオ リク リサ リツ リト リナ リホ リョウ リン リンカ
ルイ ルカ ルナ ルリ ルミ ルミコ
レイ レイカ レイコ レオ レナ レン レンカ
ロイ ロカ ロミ ロミナ ロン ワカ ワカナ ワコ ワタル
";

fn initial(name: &str) -> Option<char> {
    name.chars().next().map(|ch| ch.to_ascii_lowercase())
}

pub fn ensure_available_initial(name: &str, active: &HashSet<String>) -> Result<(), String> {
    if active.iter().any(|other| initial(other) == initial(name)) {
        return Err(format!(
            "Agent name {name}'s initial is occupied; close the conflicting Agent window and retry"
        ));
    }
    Ok(())
}

fn choose(
    language: &str,
    used: &HashSet<String>,
    active: &HashSet<String>,
) -> Result<String, String> {
    let catalog = if language == "ja" { JA_NAMES } else { EN_NAMES };
    let eligible: Vec<_> = catalog
        .split_whitespace()
        .filter(|name| !active.iter().any(|other| initial(other) == initial(name)))
        .collect();
    if eligible.is_empty() {
        return Err(
            "All Agent name initials are occupied; close an inactive Agent window and retry".into(),
        );
    }
    let fresh: Vec<_> = eligible
        .iter()
        .copied()
        .filter(|name| !used.contains(&name.to_lowercase()))
        .collect();
    let candidates = if fresh.is_empty() { &eligible } else { &fresh };
    // The first UUID field is 32 random bits (it has no version/variant bits).
    // Rejection sampling avoids modulo bias for catalogs of any size.
    let count = candidates.len() as u32;
    let limit = u32::MAX - u32::MAX % count;
    let index = loop {
        let random = uuid::Uuid::new_v4().as_fields().0;
        if random < limit {
            break (random % count) as usize;
        }
    };
    let base = candidates[index];
    let mut name = base.to_string();
    let mut suffix = 2usize;
    while used.contains(&name.to_lowercase()) {
        name = format!("{base}-{suffix}");
        suffix += 1;
    }
    Ok(name)
}

fn locked_history(path: &Path) -> Result<(std::fs::File, HashSet<String>), String> {
    let parent = path.parent().ok_or("Agent name history has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.try_lock_exclusive()
        .map_err(|error| format!("Agent name history is busy: {error}; retry assignment"))?;
    let mut used = HashSet::new();
    for line in BufReader::new(&file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let name: String = serde_json::from_str(&line)
            .map_err(|error| format!("Agent name history is corrupt: {error}"))?;
        if name.trim().is_empty() {
            return Err("Agent name history contains an empty identity".into());
        }
        used.insert(name.to_lowercase());
    }
    Ok((file, used))
}

fn reserve(
    file: &mut std::fs::File,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<bool, String> {
    if !used.insert(name.to_lowercase()) {
        return Ok(false);
    }
    serde_json::to_writer(&mut *file, name).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    Ok(true)
}

/// Seed pre-upgrade/restored names before a window can disappear.
pub fn remember(history_path: &Path, names: &HashSet<String>) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    let (mut file, mut used) = locked_history(history_path)?;
    let mut changed = false;
    for name in names {
        changed |= reserve(&mut file, &mut used, name)?;
    }
    if changed {
        file.sync_all().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Reserve the name durably before publishing it. Busy/corrupt history fails
/// closed; callers may retry without inventing or reusing a visible identity.
pub fn allocate(
    history_path: &Path,
    language: &str,
    active: &HashSet<String>,
    known: &HashSet<String>,
) -> Result<String, String> {
    let (mut file, mut used) = locked_history(history_path)?;
    for name in known {
        reserve(&mut file, &mut used, name)?;
    }
    let name = choose(language, &used, active)?;
    reserve(&mut file, &mut used, &name)?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localized_names_are_never_reused_and_initials_are_distributed() {
        for language in ["ja", "en"] {
            let catalog: Vec<_> = if language == "ja" { JA_NAMES } else { EN_NAMES }
                .split_whitespace()
                .collect();
            assert!(catalog.len() >= 200);
            assert_eq!(catalog.len(), catalog.iter().collect::<HashSet<_>>().len());
            let temp = tempfile::tempdir().unwrap();
            let history = temp.path().join("names.jsonl");
            let mut used = HashSet::new();
            let mut initials = std::collections::HashMap::new();
            for _ in 0..1000 {
                let name = allocate(&history, language, &HashSet::new(), &HashSet::new()).unwrap();
                if language == "ja" {
                    assert!(name.chars().next().unwrap() >= 'ァ', "{name}");
                } else {
                    assert!(name.starts_with(|c: char| c.is_ascii_uppercase()), "{name}");
                }
                *initials.entry(name.chars().next().unwrap()).or_insert(0) += 1;
                assert!(used.insert(name), "reused a historical identity");
            }
            assert!(*initials.values().max().unwrap() <= 150, "{initials:?}");
        }
    }

    #[test]
    fn active_initials_and_known_legacy_names_are_reserved() {
        let temp = tempfile::tempdir().unwrap();
        let history = temp.path().join("names.jsonl");
        let mut active = HashSet::new();
        let known = HashSet::from(["Unassigned".into()]);
        for _ in 0..10 {
            let name = allocate(&history, "en", &active, &known).unwrap();
            assert!(!known.contains(&name));
            assert!(!active
                .iter()
                .any(|old: &String| old.starts_with(name.chars().next().unwrap())));
            active.insert(name);
        }
        // Reopening the ledger after every allocation models separate launches.
        let name = allocate(&history, "unknown", &HashSet::new(), &HashSet::new()).unwrap();
        assert!(!active.contains(&name));
        assert!(!known.contains(&name));
    }

    #[test]
    fn corrupt_or_busy_history_refuses_without_reusing_an_identity() {
        use fs2::FileExt;
        let temp = tempfile::tempdir().unwrap();
        let history = temp.path().join("names.jsonl");
        std::fs::write(&history, "broken record\n").unwrap();
        assert!(allocate(&history, "en", &HashSet::new(), &HashSet::new()).is_err());
        std::fs::write(&history, "").unwrap();
        let lock = std::fs::File::open(&history).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(allocate(&history, "en", &HashSet::new(), &HashSet::new()).is_err());
    }
}
