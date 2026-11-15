//! Arrastar e soltar dentro do cofre: itens para pastas e pastas para dentro
//! de outras pastas.
//!
//! No Bitwarden a hierarquia de pastas é só o nome: "Trabalho/Clientes" fica
//! dentro de "Trabalho". Mover uma pasta é renomear ela e as subpastas.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::vault::Folder;

/// Um item da lista sendo arrastado.
#[derive(Clone, Debug)]
pub struct DraggedItem {
    pub id: String,
    pub name: String,
    pub icon: IconName,
}

/// Uma pasta da barra lateral sendo arrastada.
#[derive(Clone, Debug)]
pub struct DraggedFolder {
    pub id: String,
    pub name: String,
}

/// O que aparece embaixo do cursor enquanto arrasta.
pub struct DragPreview {
    icon: IconName,
    label: SharedString,
}

impl DragPreview {
    pub fn item(item: &DraggedItem, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            icon: item.icon,
            label: item.name.clone().into(),
        })
    }

    pub fn folder(folder: &DraggedFolder, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            icon: IconName::Folder,
            label: folder.name.clone().into(),
        })
    }
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .px_2p5()
            .py_1()
            .max_w(px(240.))
            .rounded(theme.radius)
            .border_1()
            .border_color(crate::appearance::accent(cx))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .text_sm()
            .shadow_md()
            .opacity(0.95)
            .child(Icon::new(self.icon).small())
            .child(div().truncate().child(self.label.clone()))
    }
}

/// Uma linha de pasta na barra lateral, já com o recuo da hierarquia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRow {
    pub id: String,
    /// Nome mostrado (sem o caminho da pasta-mãe).
    pub label: String,
    /// Quantas pastas-mãe existem acima.
    pub depth: usize,
}

/// Pastas em ordem de árvore. Um nome "A/B" fica dentro de "A" quando "A"
/// existe; se não existe, aparece com o nome inteiro.
pub fn folder_rows(folders: &[Folder]) -> Vec<FolderRow> {
    let mut sorted: Vec<&Folder> = folders.iter().collect();
    // Ordena por segmento ("A/B" logo depois de "A", antes de "A B").
    sorted.sort_by_key(|f| {
        f.name
            .split('/')
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
    });
    sorted
        .into_iter()
        .map(|f| {
            let (depth, label) = place_of(folders, f);
            FolderRow {
                id: f.id.clone(),
                label,
                depth,
            }
        })
        .collect()
}

/// (profundidade, nome relativo à pasta-mãe mais próxima que existe)
fn place_of(folders: &[Folder], folder: &Folder) -> (usize, String) {
    let name = folder.name.as_str();
    let mut depth = 0;
    let mut parent_len = None;
    for (i, _) in name.match_indices('/') {
        let prefix = &name[..i];
        if folders
            .iter()
            .any(|f| f.id != folder.id && f.name == prefix)
        {
            depth += 1;
            parent_len = Some(i);
        }
    }
    let label = match parent_len {
        Some(i) => name[i + 1..].to_owned(),
        None => name.to_owned(),
    };
    (depth, label)
}

/// Por que uma pasta não pode ir para onde foi solta.
#[derive(Debug, PartialEq, Eq)]
pub enum MoveError {
    IntoItself,
    NameTaken(String),
}

impl MoveError {
    pub fn message(&self) -> String {
        match self {
            Self::IntoItself => "Uma pasta não pode ir para dentro dela mesma.".into(),
            Self::NameTaken(name) => format!("Já existe uma pasta \"{name}\" lá."),
        }
    }
}

/// Novos nomes para mover a pasta `moving` (com as subpastas) para dentro de
/// `target` (`None` = topo). Lista vazia: já está lá.
pub fn move_folder(
    folders: &[Folder],
    moving: &str,
    target: Option<&str>,
) -> Result<Vec<(String, String)>, MoveError> {
    let Some(folder) = folders.iter().find(|f| f.id == moving) else {
        return Ok(Vec::new());
    };
    let old = folder.name.as_str();
    let target_name = match target {
        Some(t) => match folders.iter().find(|f| f.id == t) {
            Some(f) => Some(f.name.as_str()),
            None => return Ok(Vec::new()),
        },
        None => None,
    };
    if let Some(t) = target_name {
        if t == old || t.starts_with(&format!("{old}/")) {
            return Err(MoveError::IntoItself);
        }
    }
    let (_, label) = place_of(folders, folder);
    let new = match target_name {
        Some(t) => format!("{t}/{label}"),
        None => label,
    };
    if new == old {
        return Ok(Vec::new());
    }
    let child_prefix = format!("{old}/");
    let mut plan = vec![(folder.id.clone(), new.clone())];
    for f in folders {
        if let Some(rest) = f.name.strip_prefix(&child_prefix) {
            plan.push((f.id.clone(), format!("{new}/{rest}")));
        }
    }
    let moved: Vec<&str> = plan.iter().map(|(id, _)| id.as_str()).collect();
    for (_, name) in &plan {
        if folders
            .iter()
            .any(|f| !moved.contains(&f.id.as_str()) && f.name == *name)
        {
            return Err(MoveError::NameTaken(name.clone()));
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::{folder_rows, move_folder, MoveError};
    use scout_vault_core::vault::Folder;

    fn f(id: &str, name: &str) -> Folder {
        Folder {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn rows_follow_the_tree() {
        let folders = vec![
            f("c", "Trabalho/Clientes"),
            f("p", "Pessoal"),
            f("t", "Trabalho"),
            f("x", "Sem/Mãe"),
            f("a", "Trabalho/Clientes/Acme"),
        ];
        let rows: Vec<_> = folder_rows(&folders)
            .into_iter()
            .map(|r| (r.id, r.label, r.depth))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("p".into(), "Pessoal".into(), 0),
                ("x".into(), "Sem/Mãe".into(), 0),
                ("t".into(), "Trabalho".into(), 0),
                ("c".into(), "Clientes".into(), 1),
                ("a".into(), "Acme".into(), 2),
            ]
        );
    }

    #[test]
    fn moving_a_folder_takes_its_subfolders() {
        let folders = vec![
            f("t", "Trabalho"),
            f("c", "Trabalho/Clientes"),
            f("a", "Trabalho/Clientes/Acme"),
            f("p", "Pessoal"),
        ];
        assert_eq!(
            move_folder(&folders, "c", Some("p")).unwrap(),
            vec![
                ("c".into(), "Pessoal/Clientes".into()),
                ("a".into(), "Pessoal/Clientes/Acme".into()),
            ]
        );
        // Para o topo.
        assert_eq!(
            move_folder(&folders, "a", None).unwrap(),
            vec![("a".into(), "Acme".into())]
        );
        // Já está lá.
        assert!(move_folder(&folders, "c", Some("t")).unwrap().is_empty());
        assert!(move_folder(&folders, "p", None).unwrap().is_empty());
    }

    #[test]
    fn refuses_cycles_and_name_clashes() {
        let folders = vec![
            f("t", "Trabalho"),
            f("c", "Trabalho/Clientes"),
            f("p", "Pessoal"),
            f("pc", "Pessoal/Clientes"),
        ];
        assert_eq!(
            move_folder(&folders, "t", Some("c")),
            Err(MoveError::IntoItself)
        );
        assert_eq!(
            move_folder(&folders, "t", Some("t")),
            Err(MoveError::IntoItself)
        );
        assert_eq!(
            move_folder(&folders, "c", Some("p")),
            Err(MoveError::NameTaken("Pessoal/Clientes".into()))
        );
    }
}
