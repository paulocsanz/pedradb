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
}

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
    /// Cria uma nova página de memória na geração 1.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            active_generation: AtomicU64::new(1),
            buffer: vec![0u8; capacity],
        }
    }

    /// Recicla a página para uma nova geração (incrementando atomicamente o contador).
    pub fn recycle_page(&mut self) -> u64 {
        // Zera o conteúdo e avança a geração
        self.buffer.fill(0);
        self.active_generation.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Arena de memória de blocos/nós com proteção generacional contra o problema ABA.
pub struct GenerationalArena {
    pages: Vec<ArenaMemoryPage>,
}

impl GenerationalArena {
    /// Constrói a arena com um número inicial de páginas e capacidade.
    pub fn new(num_pages: usize, page_capacity: usize) -> Self {
        let mut pages = Vec::with_capacity(num_pages);
        for _ in 0..num_pages {
            pages.push(ArenaMemoryPage::new(page_capacity));
        }
        Self { pages }
    }

    /// Aloca uma fatia de bytes na página especificada e retorna um `GenerationalPtr`.
    pub fn allocate_at(
        &mut self,
        page_index: usize,
        offset: usize,
        data: &[u8],
    ) -> Result<GenerationalPtr, AbaMemoryHazardViolation> {
        let page = &mut self.pages[page_index];
        if offset + data.len() > page.capacity {
            return Err(AbaMemoryHazardViolation::PageOffsetOutOfBounds {
                offset: offset + data.len(),
                page_capacity: page.capacity,
            });
        }

        page.buffer[offset..offset + data.len()].copy_from_slice(data);
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

        if ptr.offset + len > page.capacity {
            return Err(AbaMemoryHazardViolation::PageOffsetOutOfBounds {
                offset: ptr.offset + len,
                page_capacity: page.capacity,
            });
        }

        Ok(&page.buffer[ptr.offset..ptr.offset + len])
    }

    /// Recicla uma página da arena após o término do flush da MemTable correspondente.
    pub fn recycle_page(&mut self, page_index: usize) -> u64 {
        self.pages[page_index].recycle_page()
    }
}
