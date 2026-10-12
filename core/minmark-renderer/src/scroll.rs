//! Sincronización de scroll por posición semántica (§9.1).
//!
//! «El scroll estará sincronizado bidireccionalmente mediante posición
//! semántica, no mediante porcentaje bruto de desplazamiento». El mapa de
//! anclas relaciona cada elemento de nivel superior de la Preview (orden del
//! DOM, marcado con `data-mm-span`) con su rango en el fuente: el editor
//! consulta `anchor_for_offset` y la Preview `offset_for_anchor`, y el
//! doble clic traduce rango → posición con `Editor::position_of`.

use minmark_document::{Ast, Span};

/// Ancla de sincronización: elemento de nivel superior de la Preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    /// Índice del elemento en la AST (y en el orden del DOM).
    pub block_index: usize,
    /// Rango del elemento en el fuente.
    pub span: Span,
}

/// Mapa de sincronización Editor ↔ Preview (§9.1).
#[derive(Debug, Clone, Default)]
pub struct ScrollMap {
    anchors: Vec<Anchor>,
}

impl ScrollMap {
    /// Construye el mapa desde la AST, en orden de aparición (= orden DOM).
    pub fn for_ast(ast: &Ast) -> Self {
        let anchors = ast
            .blocks
            .iter()
            .enumerate()
            .map(|(block_index, block)| Anchor {
                block_index,
                span: block.span,
            })
            .collect();
        Self { anchors }
    }

    /// Editor → Preview: elemento cuyo rango contiene `offset`. Para un offset
    /// en el espacio entre elementos devuelve el anterior (la posición
    /// semántica más próxima visible).
    pub fn anchor_for_offset(&self, offset: u32) -> Option<usize> {
        if self.anchors.is_empty() {
            return None;
        }
        let next = self
            .anchors
            .partition_point(|anchor| anchor.span.start <= offset);
        Some(if next == 0 {
            0
        } else {
            self.anchors[next - 1].block_index
        })
    }

    /// Preview → Editor: offset de inicio del elemento `block_index`.
    pub fn offset_for_anchor(&self, block_index: usize) -> Option<u32> {
        self.anchors
            .get(block_index)
            .map(|anchor| anchor.span.start)
    }

    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_parser::parse;

    #[test]
    fn mapeo_bidireccional_por_elemento() {
        let source = "# Título\n\ntexto uno\n\ntexto dos\n";
        let ast = parse(source);
        let map = ScrollMap::for_ast(&ast);
        assert_eq!(map.len(), 3);

        // Editor → Preview: offset dentro del segundo bloque.
        let offset = source.find("texto uno").unwrap() as u32;
        assert_eq!(map.anchor_for_offset(offset), Some(1));
        let offset = source.find("texto dos").unwrap() as u32 + 3;
        assert_eq!(map.anchor_for_offset(offset), Some(2));

        // Preview → Editor: inicio del elemento.
        assert_eq!(
            map.offset_for_anchor(1),
            Some(source.find("texto uno").unwrap() as u32)
        );

        // Offset en el espacio entre elementos: elemento anterior.
        let between = source.find("texto uno").unwrap() as u32 - 1;
        assert_eq!(map.anchor_for_offset(between), Some(0));
    }

    #[test]
    fn documento_vacio() {
        let map = ScrollMap::for_ast(&parse(""));
        assert!(map.is_empty());
        assert_eq!(map.anchor_for_offset(0), None);
        assert_eq!(map.offset_for_anchor(0), None);
    }
}
