//! RFC-0294: Pilar 5 - Tagging Generacional de Arenas e Imunidade ao Problema ABA.
//!
//! Vincula cada página de memória reciclada a um identificador de geração monotônico,
//! garantindo que leitores retardados nunca acessem dados de novas encarnações (imunidade ABA).

use std::sync::atomic::{AtomicU64, Ordering};

/// Violações de integridade generacional de memória e perigo ABA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbaMemoryHazardViolation {
    /// O ponteiro desreferenciado possui uma geração obsoleta (a página foi reciclada).
    StaleGenerationalReference {
        /// Índice da página física da arena.
        page_index: usize,
        /// Geração do ponteiro retido pelo leitor.
        pointer_generation: u64,
        /// Geração atual da página na arena.
        current_generation: u64,
    },
    /// O offset requisitado excedeu a capacidade da página (Out of bounds).
    PageOffsetOutOfBounds {
        /// Offset requisitado.
        offset: usize,
        /// Tamanho útil da página.
        page_capacity: usize,
    },
    /// O índice de página fornecido é inválido.
    InvalidPageIndex {
        page_index: usize,
        total_pages: usize,
    },
    /// Nenhuma página solicitada na construção da arena.
    ZeroPagesRequested,
    /// Capacidade zero de página solicitada.
    ZeroPageCapacityRequested,
    /// Geração zero é proibida como identificador válido.
    ZeroGenerationDisallowed,
    /// Contador generacional atingiu o teto máximo de u64 (prevenção de wrap-around ABA).
    GenerationOverflow,
}

impl std::fmt::Display for AbaMemoryHazardViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleGenerationalReference { page_index, pointer_generation, current_generation } => write!(
                f,
                "Stale generational reference on page {page_index}: pointer gen {pointer_generation} != active gen {current_generation}"
            ),
            Self::PageOffsetOutOfBounds { offset, page_capacity } => write!(
                f,
                "Page offset out of bounds: offset {offset} > capacity {page_capacity}"
            ),
            Self::InvalidPageIndex { page_index, total_pages } => write!(
                f,
                "Invalid page index {page_index} (total pages: {total_pages})"
            ),
            Self::ZeroPagesRequested => write!(f, "Cannot construct GenerationalArena with 0 pages"),
            Self::ZeroPageCapacityRequested => write!(f, "Cannot construct GenerationalArena with 0 page capacity"),
            Self::ZeroGenerationDisallowed => write!(f, "Generation 0 is an illegal uninitialized sentinel"),
            Self::GenerationOverflow => write!(f, "Generation counter overflow prevented (u64::MAX)"),
        }
    }
}

impl std::error::Error for AbaMemoryHazardViolation {}

/// Ponteiro generacional imutável para um nó alocado na arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationalPtr {
    /// ID de geração no momento da alocação.
    pub generation_id: u64,
    /// Índice da página física na arena.
    pub page_index: usize,
    /// Offset em bytes dentro da página.
    pub offset: usize,
}

impl GenerationalPtr {
    /// Constrói um ponteiro generacional validando a geração não-nula.
    pub fn try_new(
        generation_id: u64,
        page_index: usize,
        offset: usize,
    ) -> Result<Self, AbaMemoryHazardViolation> {
        if generation_id == 0 {
            return Err(AbaMemoryHazardViolation::ZeroGenerationDisallowed);
        }
        Ok(Self {
            generation_id,
            page_index,
            offset,
        })
    }
}

/// Página de memória física gerenciada pela arena com contador generacional atômico.
pub struct ArenaMemoryPage {
    /// Capacidade total em bytes da página (ex: 64 KiB).
    pub capacity: usize,
    /// Contador atômico da geração atual da página.
    pub active_generation: AtomicU64,
    /// Buffer de dados brutos da página.
    pub buffer: Vec<u8>,
}

impl ArenaMemoryPage {
    /// Cria uma nova página de memória na geração 1 com validação de capacidade.
    pub fn try_new(capacity: usize) -> Result<Self, AbaMemoryHazardViolation> {
        if capacity == 0 {
            return Err(AbaMemoryHazardViolation::ZeroPageCapacityRequested);
        }
        Ok(Self {
            capacity,
            active_generation: AtomicU64::new(1),
            buffer: vec![0u8; capacity],
        })
    }

    /// Cria uma nova página de memória na geração 1.
    pub fn new(capacity: usize) -> Self {
        Self::try_new(capacity).unwrap_or_else(|_| Self {
            capacity: 1,
            active_generation: AtomicU64::new(1),
            buffer: vec![0u8; 1],
        })
    }

    /// Recicla a página com proteção contra overflow de u64 (prevenção de wrap-around ABA).
    pub fn try_recycle_page(&mut self) -> Result<u64, AbaMemoryHazardViolation> {
        let current = self.active_generation.load(Ordering::SeqCst);
        if current == u64::MAX {
            return Err(AbaMemoryHazardViolation::GenerationOverflow);
        }
        self.buffer.fill(0);
        let next = current + 1;
        self.active_generation.store(next, Ordering::SeqCst);
        Ok(next)
    }

    /// Recicla a página para uma nova geração (incrementando atomicamente o contador).
    pub fn recycle_page(&mut self) -> u64 {
        self.try_recycle_page().unwrap_or(u64::MAX)
    }
}

/// Arena de memória de blocos/nós com proteção generacional contra o problema ABA.
pub struct GenerationalArena {
    pages: Vec<ArenaMemoryPage>,
}

impl GenerationalArena {
    /// Constrói a arena com validação estrita de páginas e capacidade.
    pub fn try_new(num_pages: usize, page_capacity: usize) -> Result<Self, AbaMemoryHazardViolation> {
        if num_pages == 0 {
            return Err(AbaMemoryHazardViolation::ZeroPagesRequested);
        }
        if page_capacity == 0 {
            return Err(AbaMemoryHazardViolation::ZeroPageCapacityRequested);
        }
        let mut pages = Vec::with_capacity(num_pages);
        for _ in 0..num_pages {
            pages.push(ArenaMemoryPage::try_new(page_capacity)?);
        }
        Ok(Self { pages })
    }

    /// Constrói a arena com um número inicial de páginas e capacidade.
    pub fn new(num_pages: usize, page_capacity: usize) -> Self {
        Self::try_new(num_pages, page_capacity).unwrap_or_else(|_| Self { pages: Vec::new() })
    }

    /// Aloca uma fatia de bytes na página especificada e retorna um `GenerationalPtr`.
    pub fn allocate_at(
        &mut self,
        page_index: usize,
        offset: usize,
        data: &[u8],
    ) -> Result<GenerationalPtr, AbaMemoryHazardViolation> {
        if page_index >= self.pages.len() {
            return Err(AbaMemoryHazardViolation::InvalidPageIndex {
                page_index,
                total_pages: self.pages.len(),
            });
        }

        let page = &mut self.pages[page_index];
        let end = offset.checked_add(data.len()).ok_or(AbaMemoryHazardViolation::PageOffsetOutOfBounds {
            offset: usize::MAX,
            page_capacity: page.capacity,
        })?;

        if end > page.capacity {
            return Err(AbaMemoryHazardViolation::PageOffsetOutOfBounds {
                offset: end,
                page_capacity: page.capacity,
            });
        }

        page.buffer[offset..end].copy_from_slice(data);
        let gen = page.active_generation.load(Ordering::Acquire);

        Ok(GenerationalPtr {
            generation_id: gen,
            page_index,
            offset,
        })
    }

    /// Desreferencia com segurança um ponteiro generacional validando a época ativa da página.
    pub fn read_ptr(
        &self,
        ptr: GenerationalPtr,
        len: usize,
    ) -> Result<&[u8], AbaMemoryHazardViolation> {
        if ptr.page_index >= self.pages.len() {
            return Err(AbaMemoryHazardViolation::InvalidPageIndex {
                page_index: ptr.page_index,
                total_pages: self.pages.len(),
            });
        }

        let page = &self.pages[ptr.page_index];
        let current_gen = page.active_generation.load(Ordering::Acquire);

        // Validação formal de imunidade ABA:
        // Se a página foi reciclada, current_gen > ptr.generation_id
        if current_gen != ptr.generation_id {
            return Err(AbaMemoryHazardViolation::StaleGenerationalReference {
                page_index: ptr.page_index,
                pointer_generation: ptr.generation_id,
                current_generation: current_gen,
            });
        }

        let end = ptr.offset.checked_add(len).ok_or(AbaMemoryHazardViolation::PageOffsetOutOfBounds {
            offset: usize::MAX,
            page_capacity: page.capacity,
        })?;

        if end > page.capacity {
            return Err(AbaMemoryHazardViolation::PageOffsetOutOfBounds {
                offset: end,
                page_capacity: page.capacity,
            });
        }

        Ok(&page.buffer[ptr.offset..end])
    }

    /// Recicla uma página da arena após o término do flush da MemTable correspondente com validação.
    pub fn try_recycle_page(&mut self, page_index: usize) -> Result<u64, AbaMemoryHazardViolation> {
        if page_index >= self.pages.len() {
            return Err(AbaMemoryHazardViolation::InvalidPageIndex {
                page_index,
                total_pages: self.pages.len(),
            });
        }
        self.pages[page_index].try_recycle_page()
    }

    /// Recicla uma página da arena após o término do flush da MemTable correspondente.
    pub fn recycle_page(&mut self, page_index: usize) -> u64 {
        if let Some(page) = self.pages.get_mut(page_index) {
            page.recycle_page()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generational_arena_validation_red_to_green() {
        assert_eq!(GenerationalArena::try_new(0, 1024).err(), Some(AbaMemoryHazardViolation::ZeroPagesRequested));
        assert_eq!(GenerationalArena::try_new(10, 0).err(), Some(AbaMemoryHazardViolation::ZeroPageCapacityRequested));
        assert_eq!(GenerationalPtr::try_new(0, 0, 0), Err(AbaMemoryHazardViolation::ZeroGenerationDisallowed));

        let mut arena = GenerationalArena::try_new(2, 64).expect("valid arena");
        let ptr = arena.allocate_at(0, 0, b"hello").expect("allocated");
        assert_eq!(ptr.generation_id, 1);
        let data = arena.read_ptr(ptr, 5).expect("read");
        assert_eq!(data, b"hello");

        // Recicla página 0 -> geração passa para 2
        let new_gen = arena.try_recycle_page(0).expect("recycled");
        assert_eq!(new_gen, 2);

        // Tentativa de ler com ptr antigo (geração 1) é rejeitada com StaleGenerationalReference
        assert_eq!(
            arena.read_ptr(ptr, 5),
            Err(AbaMemoryHazardViolation::StaleGenerationalReference {
                page_index: 0,
                pointer_generation: 1,
                current_generation: 2,
            })
        );
    }

    #[test]
    fn test_recycle_overflow_protection() {
        let mut page = ArenaMemoryPage::try_new(64).expect("page");
        page.active_generation.store(u64::MAX, Ordering::SeqCst);
        assert_eq!(page.try_recycle_page(), Err(AbaMemoryHazardViolation::GenerationOverflow));
    }
}
