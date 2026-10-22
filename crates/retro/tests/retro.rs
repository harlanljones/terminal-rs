//! Fixture-driven tests: the committed 2024ARI.EVN home-file, synthetic mini
//! event files, and a full-season ARI 2024 reconciliation against Lahman.

use std::path::PathBuf;
use std::process::Command;

use sabr_retro::event::ErSource;
use sabr_retro::{BatLine, Game, Kind, Side, cut_lines, parse_event, parse_file, reconcile_teams, season_cuts};

fn fixtures() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "fixtures"].iter().collect()
}

fn sample_lahman() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "sabr-data", "sample", "lahman"].iter().collect()
}

#[test]
fn fixture_tokens_all_accounted() {
    let f = parse_file(&fixtures().join("2024ARI.EVN")).expect("every play token in 2024ARI.EVN must be accounted for");
    assert_eq!(f.games.len(), 81);
    assert!(f.games.iter().all(|g| g.date.is_some()));
}

#[test]
fn fixture_uses_data_er_records() {
    let f = parse_file(&fixtures().join("2024ARI.EVN")).unwrap();
    assert!(
        f.games.iter().all(|g| g.er_source == ErSource::DataRecords),
        "data,er records are present in 2024ARI.EVN and must be used"
    );
}

#[test]
fn radj_is_not_a_pa() {
    let text = "\
id,TEST1
info,visteam,AAA
info,hometeam,BBB
info,date,2024/04/01
start,pit1,\"P One\",1,1,1
start,b1,\"B One\",0,1,8
start,r1,\"R One\",0,2,7
play,1,0,b1,00,X,K
play,1,0,b1,00,X,WP.2-H
play,10,0,b1,00,X,K
radj,r1,2
data,er,pit1,0
";
    let dir = std::env::temp_dir().join("sabr-retro-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mini.evn");
    std::fs::write(&path, text).unwrap();
    let f = parse_file(&path).unwrap();
    assert_eq!(f.games.len(), 1);
    let b1 = &f.games[0].batting["b1"];
    // K, WP (not a PA), placed-runner-inning K: three lines, two PAs.
    assert_eq!(b1.pa, 2);
    assert_eq!(b1.so, 2);
    let r1 = f.games[0].batting.get("r1");
    assert!(r1.is_none_or(|l| l.pa == 0), "radj placed runner must not create a PA");
    // The placed runner scored on the WP after placement? Not yet: he is on
    // second. The second WP advances 2-H explicitly.
}

#[test]
fn implicit_hr_run_counted() {
    let parsed = parse_event("HR").expect("bare HR parses");
    assert_eq!(parsed.kind, Kind::Homer);
    assert!(parsed.batter_safe());
    assert_eq!(parsed.dest, 4);
    // Full file: HR scores the batter with no explicit B-H advance.
    let text = "\
id,TEST2
info,visteam,AAA
info,hometeam,BBB
info,date,2024/04/01
start,pit1,\"P One\",1,1,1
start,b1,\"B One\",0,1,8
play,1,0,b1,00,X,HR
data,er,pit1,1
";
    let dir = std::env::temp_dir().join("sabr-retro-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hr.evn");
    std::fs::write(&path, text).unwrap();
    let f = parse_file(&path).unwrap();
    let b1 = &f.games[0].batting["b1"];
    assert_eq!(b1.pa, 1);
    assert_eq!(b1.ab, 1);
    assert_eq!(b1.h, 1);
    assert_eq!(b1.hr, 1);
    assert_eq!(b1.r, 1, "implicit HR batter run counted");
    assert_eq!(b1.rbi, 1);
    let p1 = &f.games[0].pitching["pit1"];
    assert_eq!(p1.r, 1);
    assert_eq!(p1.er, 1);
}

#[test]
fn unknown_token_fails_loudly() {
    let e = parse_event("ZZ9/F8");
    match e {
        Err(sabr_retro::ParseError::UnknownToken { token, .. }) => {
            assert!(token.starts_with("ZZ9"), "error must name the token: {token}");
        }
        other => panic!("expected UnknownToken, got {other:?}"),
    }
}

#[test]
fn cut_lines_cumulative_and_exclusive() {
    // Two games: one before the May 1 cut, one after.
    let mut g1: Game = Game { date: Some(jiff::civil::Date::constant(2024, 4, 28)), ..Default::default() };
    g1.batting.insert("p1".into(), BatLine { pa: 4, ab: 4, h: 2, ..Default::default() });
    let mut g2: Game = Game { date: Some(jiff::civil::Date::constant(2024, 5, 2)), ..Default::default() };
    g2.batting.insert("p1".into(), BatLine { pa: 3, ab: 3, h: 1, hr: 1, ..Default::default() });
    let cuts = season_cuts(&[2024]);
    assert_eq!(cuts.len(), 5);
    let lines = cut_lines(&[g1, g2], &cuts);
    let may = lines.iter().find(|l| l.as_of == jiff::civil::Date::constant(2024, 4, 30)).unwrap();
    assert_eq!(may.pa, 4, "May 1 cut includes only games before it");
    let jun = lines.iter().find(|l| l.as_of == jiff::civil::Date::constant(2024, 5, 31)).unwrap();
    assert_eq!(jun.pa, 7, "later cuts accumulate");
    assert_eq!(jun.hr, 1);
}

#[test]
fn ari_2024_reconciles_against_lahman() {
    // The mirror carries one file per team per season (the team's home-only
    // or away-only block), so a batter's FULL season spans every team's file.
    // Cache all 30 regular-season files under the test target dir: strictly
    // sequential, one second apart, one UA, so the Chadwick mirror is treated
    // gently. The committed fixture (2024ARI.EVN) is validated on its own by
    // the token test above.
    const TEAMS: &[&str] = &[
        "ANA", "ARI", "ATL", "BAL", "BOS", "CHA", "CHN", "CIN", "CLE", "COL", "DET", "HOU", "KCA", "LAN", "MIA", "MIL",
        "MIN", "NYA", "NYN", "OAK", "PHI", "PIT", "SDN", "SEA", "SFN", "SLN", "TBA", "TEX", "TOR", "WAS",
    ];
    let dir = std::env::var("SABR_RETRO_CACHE")
        .map(std::path::PathBuf::from)
        .unwrap_or(PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    std::fs::create_dir_all(&dir).unwrap();
    let mut games = Vec::new();
    for team in TEAMS {
        let cached = [".EVN", ".EVA"].iter().map(|e| dir.join(format!("2024{team}{e}"))).find(|p| p.exists());
        let path = match cached {
            Some(p) => p,
            None => {
                let mut path = None;
                for ext in [".EVN", ".EVA"] {
                    let p = dir.join(format!("2024{team}{ext}"));
                    let url = format!(
                        "https://raw.githubusercontent.com/chadwickbureau/retrosheet/official/seasons/2024/2024{team}{ext}"
                    );
                    let status = Command::new("curl")
                        .args(["-sS", "--fail", "-A", "sabr-rs-build", "-o"])
                        .arg(&p)
                        .arg(&url)
                        .status()
                        .expect("curl available");
                    if status.success() {
                        path = Some(p);
                        break;
                    }
                    let _ = std::fs::remove_file(&p);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                path.expect("each team has exactly one event file on the mirror")
            }
        };
        let f = parse_file(&path).unwrap();
        games.extend(f.games);
    }
    assert!(
        games.len() >= 2400,
        "the 30 team files should cover the complete 2024 regular season, got {} games",
        games.len()
    );
    games.sort_by_key(|g| g.date);

    let r = reconcile_teams(&games, &sample_lahman(), Side::Batting, &["ARI"]).unwrap();
    println!("{r}");
    for stat in ["pa", "h", "hr", "bb", "so"] {
        let s = r.stats.iter().find(|s| s.stat == stat).unwrap();
        assert!(
            s.rate >= 0.995,
            "{stat}: {}/{} exact ({:.2}%) — below the 99.5% gate",
            s.exact,
            s.compared,
            100.0 * s.rate
        );
    }
    // Pitchers too: data,er path with a loose look at printed rates.
    let p = reconcile_teams(&games, &sample_lahman(), Side::Pitching, &["ARI"]).unwrap();
    println!("{p}");
}
