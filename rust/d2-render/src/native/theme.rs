// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Theme colors and fill rules ported from D2 (d2themes, d2themescatalog,
// d2graph GetFill/GetStroke), Copyright 2022 Terrastruct, Inc.

//! D2's theme catalog.

use super::graph::Graph;

/// One D2 theme.
#[derive(Debug)]
pub struct Theme {
    /// Theme id, as passed to `--theme`.
    pub id: i64,
    /// Catalog name.
    pub name: &'static str,
    /// Neutrals N1..N7.
    pub n: [&'static str; 7],
    /// Base colors B1..B6.
    pub b: [&'static str; 6],
    /// Alternative A colors AA2, AA4, AA5.
    pub aa: [&'static str; 3],
    /// Alternative B colors AB4, AB5.
    pub ab: [&'static str; 2],
    /// Special rules (`Mono`, `CapsLock`, ...).
    pub special: &'static [&'static str],
}

/// All themes D2 0.7 ships, light then dark.
pub const THEMES: &[Theme] = &[
    Theme {
        id: 0,
        name: "Neutral Default",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#0D32B2", "#0D32B2", "#E3E9FD", "#E3E9FD", "#EDF0FD", "#F7F8FE",
        ],
        aa: ["#4A6FF3", "#EDF0FD", "#F7F8FE"],
        ab: ["#EDF0FD", "#F7F8FE"],
        special: &[],
    },
    Theme {
        id: 1,
        name: "Neutral Grey",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8",
        ],
        aa: ["#676C7E", "#CFD2DD", "#DEE1EB"],
        ab: ["#CFD2DD", "#DEE1EB"],
        special: &[],
    },
    Theme {
        id: 3,
        name: "Flagship Terrastruct",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#000E3D", "#234CDA", "#6B8AFB", "#A6B8F8", "#D2DBFD", "#E7EAFF",
        ],
        aa: ["#5829DC", "#B4AEF8", "#E4DBFF"],
        ab: ["#7FDBF8", "#C3F0FF"],
        special: &[],
    },
    Theme {
        id: 4,
        name: "Cool Classics",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#000536", "#0F66B7", "#4393DD", "#87BFF3", "#BCDDFB", "#E5F3FF",
        ],
        aa: ["#076F6F", "#77DEDE", "#C3F8F8"],
        ab: ["#C1A2F3", "#DACEFB"],
        special: &[],
    },
    Theme {
        id: 5,
        name: "Mixed Berry Blue",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#000536", "#0F66B7", "#4393DD", "#87BFF3", "#BCDDFB", "#E5F3FF",
        ],
        aa: ["#7639C5", "#C1A2F3", "#DACEFB"],
        ab: ["#EA99C6", "#FFDEF1"],
        special: &[],
    },
    Theme {
        id: 6,
        name: "Grape Soda",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#170034", "#7639C5", "#8F70D1", "#C1A2F3", "#DACEFB", "#F2EDFF",
        ],
        aa: ["#0F66B7", "#87BFF3", "#BCDDFB"],
        ab: ["#EA99C6", "#FFDAEF"],
        special: &[],
    },
    Theme {
        id: 7,
        name: "Aubergine",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#170034", "#7639C5", "#8F70D1", "#D0B9F5", "#E7DEFF", "#F4F0FF",
        ],
        aa: ["#0F66B7", "#87BFF3", "#BCDDFB"],
        ab: ["#92E3E3", "#D7F5F5"],
        special: &[],
    },
    Theme {
        id: 8,
        name: "Colorblind Clear",
        n: [
            "#0A0F25", "#676C7E", "#9499AB", "#CFD2DD", "#DEE1EB", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#010E31", "#173688", "#5679D4", "#84A1EC", "#C8D6F9", "#E5EDFF",
        ],
        aa: ["#048E63", "#A6E2D0", "#CAF2E6"],
        ab: ["#FFDA90", "#FFF0D1"],
        special: &[],
    },
    Theme {
        id: 100,
        name: "Vanilla Nitro Cola",
        n: [
            "#170206", "#535152", "#787777", "#CCCACA", "#DFDCDC", "#ECEBEB", "#FFFFFF",
        ],
        b: [
            "#1E1303", "#55452F", "#9A876C", "#C9B9A1", "#E9DBCA", "#FAF1E6",
        ],
        aa: ["#D35F0A", "#FABA8A", "#FFE0C7"],
        ab: ["#84A1EC", "#D5E0FD"],
        special: &[],
    },
    Theme {
        id: 101,
        name: "Orange Creamsicle",
        n: [
            "#170206", "#535152", "#787777", "#CCCACA", "#DFDCDC", "#ECEBEB", "#FFFFFF",
        ],
        b: [
            "#311602", "#D35F0A", "#F18F47", "#FABA8A", "#FFE0C7", "#FFF6EF",
        ],
        aa: ["#13A477", "#A6E2D0", "#CAF2E6"],
        ab: ["#FEEC8C", "#FFF8CF"],
        special: &[],
    },
    Theme {
        id: 102,
        name: "Shirley Temple",
        n: [
            "#170206", "#535152", "#787777", "#CCCACA", "#DFDCDC", "#ECEBEB", "#FFFFFF",
        ],
        b: [
            "#31021D", "#9B1A48", "#D2517F", "#EA99B6", "#FFDAE7", "#FCEDF2",
        ],
        aa: ["#D35F0A", "#FABA8A", "#FFE0C7"],
        ab: ["#FFE767", "#FFF2AA"],
        special: &[],
    },
    Theme {
        id: 103,
        name: "Earth Tones",
        n: [
            "#170206", "#535152", "#787777", "#CCCACA", "#DFDCDC", "#ECEBEB", "#FFFFFF",
        ],
        b: [
            "#1E1303", "#55452F", "#9A876C", "#C9B9A1", "#E9DBCA", "#FAF1E6",
        ],
        aa: ["#D35F0A", "#FABA8A", "#FFE0C7"],
        ab: ["#FFE767", "#FFF2AA"],
        special: &[],
    },
    Theme {
        id: 104,
        name: "Everglade Green",
        n: [
            "#170206", "#535152", "#787777", "#CCCACA", "#DFDCDC", "#ECEBEB", "#FFFFFF",
        ],
        b: [
            "#023324", "#048E63", "#49BC99", "#A6E2D0", "#CAF2E6", "#EBFDF7",
        ],
        aa: ["#D35F0A", "#FABA8A", "#FFE0C7"],
        ab: ["#C9B9A1", "#E9DBCA"],
        special: &[],
    },
    Theme {
        id: 105,
        name: "Buttered Toast",
        n: [
            "#170206", "#535152", "#787777", "#CCCACA", "#DFDCDC", "#ECEBEB", "#FFFFFF",
        ],
        b: [
            "#312102", "#DF9C18", "#FDC659", "#FFDA90", "#FFF0D1", "#FFF7E7",
        ],
        aa: ["#55452F", "#C9B9A1", "#E9DBCA"],
        ab: ["#FABA8A", "#FFE0C7"],
        special: &[],
    },
    Theme {
        id: 300,
        name: "Terminal",
        n: [
            "#000410", "#0000B8", "#9499AB", "#CFD2DD", "#C3DEF3", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#000410", "#0000E4", "#5AA4DC", "#E7E9EE", "#F5F6F9", "#FFFFFF",
        ],
        aa: ["#008566", "#45BBA5", "#7ACCBD"],
        ab: ["#F1C759", "#F9E088"],
        special: &[
            "Mono",
            "NoCornerRadius",
            "OuterContainerDoubleBorder",
            "ContainerDots",
            "CapsLock",
        ],
    },
    Theme {
        id: 301,
        name: "Terminal Grayscale",
        n: [
            "#000410", "#000410", "#9499AB", "#FFFFFF", "#FFFFFF", "#EEF1F8", "#FFFFFF",
        ],
        b: [
            "#000410", "#000410", "#FFFFFF", "#E7E9EE", "#F5F6F9", "#FFFFFF",
        ],
        aa: ["#6D7284", "#F5F6F9", "#FFFFFF"],
        ab: ["#F5F6F9", "#FFFFFF"],
        special: &[
            "Mono",
            "NoCornerRadius",
            "OuterContainerDoubleBorder",
            "ContainerDots",
            "CapsLock",
        ],
    },
    Theme {
        id: 302,
        name: "Origami",
        n: [
            "#170206", "#6F0019", "#FFFFFF", "#E07088", "#D2B098", "#FFFFFF", "#FFFFFF",
        ],
        b: [
            "#170206", "#A62543", "#E07088", "#F3E0D2", "#FAF1E6", "#FFFBF8",
        ],
        aa: ["#0A4EA6", "#3182CD", "#68A8E4"],
        ab: ["#E07088", "#F19CAE"],
        special: &["NoCornerRadius", "OuterContainerDoubleBorder", "AllPaper"],
    },
    Theme {
        id: 303,
        name: "C4",
        n: [
            "#0f5eaa", "#707070", "#FFFFFF", "#073b6f", "#999999", "#FFFFFF", "#FFFFFF",
        ],
        b: [
            "#073b6f", "#08427b", "#3c7fc0", "#438dd5", "#8a8a8a", "#999999",
        ],
        aa: ["#0f5eaa", "#707070", "#f5f5f5"],
        ab: ["#e1e1e1", "#f0f0f0"],
        special: &["C4"],
    },
    Theme {
        id: 200,
        name: "Dark Mauve",
        n: [
            "#CDD6F4", "#BAC2DE", "#A6ADC8", "#585B70", "#45475A", "#313244", "#1E1E2E",
        ],
        b: [
            "#CBA6f7", "#CBA6f7", "#6C7086", "#585B70", "#45475A", "#313244",
        ],
        aa: ["#f38BA8", "#45475A", "#313244"],
        ab: ["#45475A", "#313244"],
        special: &[],
    },
    Theme {
        id: 201,
        name: "Dark Flagship Terrastruct",
        n: [
            "#F4F6FA", "#BBBEC9", "#868A96", "#676D7D", "#3A3D49", "#191C28", "#000410",
        ],
        b: [
            "#F4F6FA", "#6B8AFB", "#3733E9", "#070B67", "#0B1197", "#3733E9",
        ],
        aa: ["#8B5DEE", "#4918B1", "#7240DD"],
        ab: ["#00607C", "#01799D"],
        special: &[],
    },
];

/// Theme by id.
pub fn find(id: i64) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.id == id)
}

impl Theme {
    /// Whether the theme has special rule `rule`.
    pub fn has(&self, rule: &str) -> bool {
        self.special.contains(&rule)
    }

    /// Default fill of object `i`.
    pub fn fill(&self, g: &Graph, i: usize) -> &'static str {
        let o = &g.objects[i];
        let level = o.level;
        match o.shape.as_str() {
            "rectangle" | "square" | "circle" | "oval" | "hierarchy" | "class" | "sql_table"
            | "sequence_diagram" => match level {
                1 if !o.is_container() => self.b[5],
                1 => self.b[3],
                2 => self.b[4],
                3 => self.b[5],
                _ => self.n[6],
            },
            "cylinder" | "stored_data" | "package" => {
                if level == 1 {
                    self.aa[1]
                } else {
                    self.aa[2]
                }
            }
            "step" | "page" | "document" => {
                if level == 1 {
                    self.ab[0]
                } else {
                    self.ab[1]
                }
            }
            "person" | "c4-person" => self.b[2],
            "diamond" => self.n[3],
            "queue" | "parallelogram" | "hexagon" => self.n[4],
            _ => self.n[6],
        }
    }

    /// Default stroke of object `i`.
    pub fn stroke(&self, g: &Graph, i: usize, dashed: bool) -> &'static str {
        match g.objects[i].shape.as_str() {
            "text" | "code" => self.n[0],
            _ if dashed => self.b[1],
            _ => self.b[0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_and_codes() {
        assert_eq!(THEMES.len(), 20);
        let t = find(0).unwrap();
        assert_eq!(t.name, "Neutral Default");
        assert_eq!(t.b[0], "#0D32B2");
        assert_eq!(t.aa[0], "#4A6FF3");
        assert!(find(300).unwrap().has("Mono"));
        assert!(find(2).is_none());
    }
}
