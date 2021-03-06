use device::{ProgramId, TextureId};
use internal_types::{AxisDirection, PackedVertex, PackedVertexForTextureCacheUpdate, Primitive};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};
use std::u16;

pub const MAX_MATRICES_PER_BATCH: usize = 32;
pub const MAX_TILE_PARAMS_PER_BATCH: usize = 64;       // TODO(gw): Constrain to max FS uniform vectors...
pub const INVALID_TILE_PARAM: u8 = 0;

static ID_COUNTER: AtomicUsize = ATOMIC_USIZE_INIT;

#[inline]
pub fn new_id() -> usize {
    ID_COUNTER.fetch_add(1, SeqCst)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct VertexBufferId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixIndex(pub u8);

#[derive(Clone, Debug)]
pub struct TileParams {
    pub u0: f32,
    pub v0: f32,
    pub u_size: f32,
    pub v_size: f32,
}

impl VertexBufferId {
    fn new() -> VertexBufferId {
        VertexBufferId(new_id())
    }
}

pub struct VertexBuffer {
    pub id: VertexBufferId,
    pub vertices: Vec<PackedVertex>,
    pub indices: Vec<u16>,
}

impl VertexBuffer {
    pub fn new() -> VertexBuffer {
        VertexBuffer {
            id: VertexBufferId::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct Batch {
    pub color_texture_id: TextureId,
    pub mask_texture_id: TextureId,
    pub first_vertex: u32,
    pub index_count: u16,
    pub tile_params: Vec<TileParams>,
}

impl Batch {
    pub fn new(color_texture_id: TextureId, mask_texture_id: TextureId, first_vertex: u32)
               -> Batch {
        let default_tile_params = vec![
            TileParams {
                u0: 0.0,
                v0: 0.0,
                u_size: 1.0,
                v_size: 1.0,
            }
        ];

        Batch {
            color_texture_id: color_texture_id,
            mask_texture_id: mask_texture_id,
            first_vertex: first_vertex,
            index_count: 0,
            tile_params: default_tile_params,
        }
    }

    pub fn can_add_to_batch(&self,
                            color_texture_id: TextureId,
                            mask_texture_id: TextureId,
                            index_count: u16,
                            needs_tile_params: bool) -> bool {
        let color_texture_ok = color_texture_id == self.color_texture_id;
        let mask_texture_ok = mask_texture_id == self.mask_texture_id;
        let index_count_ok = (self.index_count as u32 + index_count as u32) < u16::MAX as u32;
        let tile_params_ok = !needs_tile_params ||
                             self.tile_params.len() < MAX_TILE_PARAMS_PER_BATCH;

        color_texture_ok && mask_texture_ok && index_count_ok && tile_params_ok
    }

    pub fn add_draw_item(&mut self, index_count: u16, tile_params: Option<TileParams>) -> u8 {
        //println!("index_count before={} after={}", index_count, self.index_count + index_count);
        self.index_count += index_count;

        tile_params.map_or(INVALID_TILE_PARAM, |tile_params| {
            let index = self.tile_params.len();
            debug_assert!(index < MAX_TILE_PARAMS_PER_BATCH);
            self.tile_params.push(tile_params);
            index as u8
        })
    }
}

pub struct BatchBuilder<'a> {
    vertex_buffer: &'a mut VertexBuffer,
    batches: Vec<Batch>,
    current_matrix_index: u8,
}

impl<'a> BatchBuilder<'a> {
    pub fn new(vertex_buffer: &mut VertexBuffer) -> BatchBuilder {
        BatchBuilder {
            vertex_buffer: vertex_buffer,
            batches: Vec::new(),
            current_matrix_index: 0,
        }
    }

    pub fn finalize(self) -> Vec<Batch> {
        self.batches
    }

    pub fn next_draw_list(&mut self) {
        debug_assert!((self.current_matrix_index as usize) < MAX_MATRICES_PER_BATCH);
        self.current_matrix_index += 1;
    }

    pub fn add_draw_item(&mut self,
                         color_texture_id: TextureId,
                         mask_texture_id: TextureId,
                         primitive: Primitive,
                         vertices: &mut [PackedVertex],
                         tile_params: Option<TileParams>) {
        let index_count = match primitive {
            Primitive::Rectangles | Primitive::Glyphs => {
                (vertices.len() / 4 * 6) as u16
            }
            Primitive::Triangles => vertices.len() as u16,
            Primitive::TriangleFan => ((vertices.len() - 2) * 3) as u16,
        };

        let need_new_batch = match self.batches.last_mut() {
            Some(batch) => {
                !batch.can_add_to_batch(color_texture_id,
                                        mask_texture_id,
                                        index_count,
                                        tile_params.is_some())
            }
            None => {
                true
            }
        };

        let index_offset = self.vertex_buffer.vertices.len();

        if need_new_batch {
            self.batches.push(Batch::new(color_texture_id,
                                         mask_texture_id,
                                         self.vertex_buffer.indices.len() as u32));
        }

        match primitive {
            Primitive::Rectangles | Primitive::Glyphs => {
                for i in (0..vertices.len()).step_by(4) {
                    let index_base = (index_offset + i) as u16;
                    debug_assert!(index_base as usize == index_offset + i);
                    self.vertex_buffer.indices.push(index_base + 0);
                    self.vertex_buffer.indices.push(index_base + 1);
                    self.vertex_buffer.indices.push(index_base + 2);
                    self.vertex_buffer.indices.push(index_base + 2);
                    self.vertex_buffer.indices.push(index_base + 3);
                    self.vertex_buffer.indices.push(index_base + 1);
                }
            }
            Primitive::Triangles => {
                for i in (0..vertices.len()).step_by(3) {
                    let index_base = (index_offset + i) as u16;
                    debug_assert!(index_base as usize == index_offset + i);
                    self.vertex_buffer.indices.push(index_base + 0);
                    self.vertex_buffer.indices.push(index_base + 1);
                    self.vertex_buffer.indices.push(index_base + 2);
                }
            }
            Primitive::TriangleFan => {
                for i in 1..vertices.len() - 1 {
                    self.vertex_buffer.indices.push(index_offset as u16);        // center vertex
                    self.vertex_buffer.indices.push((index_offset + i + 0) as u16);
                    self.vertex_buffer.indices.push((index_offset + i + 1) as u16);
                }
            }
        }

        let tile_params_index = self.batches.last_mut().unwrap().add_draw_item(index_count,
                                                                               tile_params);

        for vertex in vertices.iter_mut() {
            vertex.matrix_index = self.current_matrix_index;
            vertex.tile_params_index = tile_params_index;
        }

        self.vertex_buffer.vertices.extend_from_slice(vertices);

        // TODO(gw): Handle exceeding u16 index buffer!
        debug_assert!(self.vertex_buffer.vertices.len() < 65535);
    }
}

/// A batch for raster jobs.
pub struct RasterBatch {
    pub program_id: ProgramId,
    pub blur_direction: Option<AxisDirection>,
    pub dest_texture_id: TextureId,
    pub color_texture_id: TextureId,
    pub vertices: Vec<PackedVertexForTextureCacheUpdate>,
    pub indices: Vec<u16>,
}

impl RasterBatch {
    pub fn new(program_id: ProgramId,
               blur_direction: Option<AxisDirection>,
               dest_texture_id: TextureId,
               color_texture_id: TextureId)
               -> RasterBatch {
        debug_assert!(dest_texture_id != color_texture_id);
        RasterBatch {
            program_id: program_id,
            blur_direction: blur_direction,
            dest_texture_id: dest_texture_id,
            color_texture_id: color_texture_id,
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    pub fn can_add_to_batch(&self,
                            dest_texture_id: TextureId,
                            color_texture_id: TextureId,
                            program_id: ProgramId,
                            blur_direction: Option<AxisDirection>)
                            -> bool {
        let batch_ok = program_id == self.program_id &&
            blur_direction == self.blur_direction &&
            dest_texture_id == self.dest_texture_id &&
            color_texture_id == self.color_texture_id;
/*
        println!("batch ok? {:?} program_id={:?}/{:?} blur_direction={:?}/{:?} \
                  dest_texture_id {:?}/{:?} color_texture_id={:?}/{:?}",
                 batch_ok,
                 program_id, self.program_id,
                 blur_direction, self.blur_direction,
                 dest_texture_id, self.dest_texture_id,
                 color_texture_id, self.color_texture_id);
*/
        batch_ok
    }

    pub fn add_draw_item(&mut self,
                         dest_texture_id: TextureId,
                         color_texture_id: TextureId,
                         vertices: &[PackedVertexForTextureCacheUpdate]) {
        debug_assert!(dest_texture_id == self.dest_texture_id);
        debug_assert!(color_texture_id == self.color_texture_id);

        for i in (0..vertices.len()).step_by(4) {
            let index_offset = self.vertices.len();
            let index_base = (index_offset + i) as u16;
            self.indices.push(index_base + 0);
            self.indices.push(index_base + 1);
            self.indices.push(index_base + 2);
            self.indices.push(index_base + 2);
            self.indices.push(index_base + 3);
            self.indices.push(index_base + 1);
        }

        self.vertices.extend_from_slice(vertices);
    }
}

/*
#[derive(Debug)]
pub struct CompositeBatch {
    op: CompositionOp,
    // TODO(gw): Convert these to a vertex buffer in backend thread...
    rects: Vec<Rect<i32>>,
}

impl CompositeBatch {
    fn new(op: CompositionOp) -> CompositeBatch {
        CompositeBatch {
            op: op,
            rects: Vec::new(),
        }
    }

    fn can_add_to_batch(&self, op: CompositionOp) -> bool {
        self.op == op
    }

    fn add_composite_item(&mut self, rect: Rect<i32>) {
        self.rects.push(rect);
    }
}

/// A batch builder for composition operations.
#[derive(Debug)]
pub struct CompositeBatchBuilder {
    batches: Vec<CompositeBatch>,
}

impl CompositeBatchBuilder {
    pub fn new() -> CompositeBatchBuilder {
        CompositeBatchBuilder {
            batches: Vec::new(),
        }
    }

    pub fn finalize(self) -> Vec<CompositeBatch> {
        self.batches
    }

    pub fn add_composite_item(&mut self,
                              op: CompositionOp,
                              rect: Rect<i32>) {
        let need_new_batch = match self.batches.last_mut() {
            Some(batch) => {
                !batch.can_add_to_batch(op)
            }
            None => {
                true
            }
        };

        if need_new_batch {
            self.batches.push(CompositeBatch::new(op));
        }

        self.batches.last_mut().unwrap().add_composite_item(rect);
    }
}
*/