"""milox datatypes — the pure, zero-parity-risk subset of upstream
``mili.datatypes`` that the read-path test suite imports.

``Superclass``, ``Metadata`` and ``ReturnCode`` are copied **verbatim**
from ``reference/mili-python/src/mili/datatypes.py`` (only the unused
heavy dataclasses / AFile parsing scaffolding is omitted — the Rust
core owns A-file parsing). Keeping them byte-identical means
``from mili.datatypes import Superclass, Metadata`` ports unchanged.
"""

from __future__ import annotations

from enum import Enum, IntEnum
from typing import List, Tuple

import numpy as np
from numpy.typing import NDArray
from typing_extensions import NotRequired, TypeAlias, TypedDict


class Superclass(IntEnum):
    """The superclass denotes what mesh class an object belongs to."""

    M_UNIT = 0
    M_NODE = 1
    M_TRUSS = 2
    M_BEAM = 3
    M_TRI = 4
    M_QUAD = 5
    M_TET = 6
    M_PYRAMID = 7
    M_WEDGE = 8
    M_HEX = 9
    M_MAT = 10
    M_MESH = 11
    M_SURFACE = 12
    M_PARTICLE = 13
    M_TET10 = 14
    M_INODE = 15
    M_QTY_SUPERCLASS = 16
    M_SHARED = 100
    M_ALL = 200
    M_INVALID_LABEL = -1

    def node_count(self) -> int:
        """Return the node count for each superclass."""
        return [0, 0, 2, 3, 3, 4, 4, 5, 6, 8, 0, 0, 0, 1, 10, 1][self.value]

    def node_connections(self) -> NDArray[np.int32]:
        """Return the nodal connections map for each superclass."""
        node_connections = {
            Superclass.M_TRUSS: np.array([[1], [0]]),
            Superclass.M_BEAM: np.array([[1], [0]]),
            Superclass.M_TRI: np.array([[1, 2], [0, 2], [0, 1]]),
            Superclass.M_QUAD: np.array([[1, 3], [0, 2], [1, 3], [0, 2]]),
            Superclass.M_TET: np.array(
                [[1, 2, 3], [0, 2, 3], [0, 1, 3], [0, 1, 2]]
            ),
            Superclass.M_HEX: np.array(
                [
                    [1, 3, 4],
                    [0, 2, 5],
                    [1, 3, 6],
                    [0, 2, 7],
                    [0, 5, 7],
                    [1, 4, 6],
                    [2, 5, 7],
                    [3, 4, 6],
                ]
            ),
            Superclass.M_PARTICLE: np.array([[0]]),
            Superclass.M_INODE: np.array([[0]]),
        }
        if self.value not in node_connections:
            raise NotImplementedError(
                f"This function does not support the superclass {self}. "
                "Please reach out to the mili-python developers."
            )
        return node_connections[self]


class QueryLayout(TypedDict):
    """Verbatim port of upstream ``mili.datatypes.QueryLayout`` — the
    layout sub-dict of a ``query()`` result. Pure data, zero parity
    risk; the parity-correct values are produced by the Rust core."""

    states: NDArray[np.int32]
    labels: NDArray[np.int32]
    components: List[str]
    times: NDArray[np.floating]


class QueryDict(TypedDict):
    """Verbatim port of upstream ``mili.datatypes.QueryDict`` — the
    per-svar ``query()`` result shape."""

    class_name: str
    source: str
    title: str
    data: NDArray[np.floating]
    layout: QueryLayout
    modifier: NotRequired[str]


class ReturnCode(Enum):
    """Return code enum for _MiliInternal."""

    OK = 0
    ERROR = 1
    CRITICAL = 2

    def str_repr(self) -> str:
        """Get string representation of ReturnCode."""
        return ["Success", "Error", "Critical"][self.value]


ReturnCodeTuple: TypeAlias = Tuple[ReturnCode, str]


class Metadata(TypedDict):
    code_name: str
    username: str
    job_id: str
    nprocs: int
    date: str
    host_name: str
    library_version: str


# ---------------------------------------------------------------------------
# M4-followup Phase G — verbatim pure-data ports from upstream
# ``reference/mili-python/src/mili/datatypes.py`` (``flatten``,
# ``MiliType``, ``StateVariable``, ``Subrecord``, ``MeshObjectClass``).
# Copied byte-identical so the read-path suite's ``isinstance`` / ``__eq__``
# checks port unchanged; the Rust core owns all parsing and the reshape
# computation, ``milox.miliinternal`` only boxes core data into these.
# See planning/mili-py/m4.md decision 19.
# ---------------------------------------------------------------------------
import dataclasses
import warnings
from dataclasses import dataclass, field
from typing import Any, Dict, Iterable, List, Optional, Tuple, Union


def flatten( iterable: Iterable[Any] ) -> List[Any]:
  """Helper function to flatten lists and tuples.

  NOTE: Will not flatten a dictionary. Only list, tuple, and set.
  NOTE: Will recur only to the depth limit set by the environment.
  """
  result = []
  for item in iterable:
    if isinstance(item, (list,tuple,set)):
      result.extend(flatten(item))
    elif isinstance(item, str):
      result.append(item)
    else:
      try:
        iterator = iter(item)
        result.extend(flatten(iterator))
      except TypeError:
        result.append(item)
  return result

class MiliType(IntEnum):
  M_INVALID = 0
  M_STRING = 1
  M_FLOAT = 2
  M_FLOAT4 = 3
  M_FLOAT8 = 4
  M_INT = 5
  M_INT4 = 6
  M_INT8 = 7
  def byte_size(self) -> int:
    return [ -1, 1, 4, 4, 8, 4, 4, 8 ][self.value]
  def numpy_dtype(self) -> Any:
    return [ None, object, np.float32, np.float32, np.float64, np.int32, np.int32, np.int64 ][self.value]


@dataclass(eq=False)
class StateVariable:
  class Aggregation(IntEnum):
    '''
    The aggregate determines how a state variable contains other state variables
    and/or stores vectors of data.
    '''
    SCALAR = 0        # has a single value
    VECTOR = 1        # contains svar components
    ARRAY = 2         # contains an array of multiple values
    VEC_ARRAY = 3     # contains an array of vectors of svars, so each array index has a full vector as described by the vector list
    NUM_AGG_TYPES = 4

  ''' Describes the layout of an aggregation of atoms of data, see StateVariable.Aggregation for more layout info '''
  name : str = ''
  title : str = ''
  agg_type : int = Aggregation.NUM_AGG_TYPES
  data_type : MiliType = MiliType.M_INVALID
  list_size : int = 0
  order : int = 0
  dims : List[int] = dataclasses.field(default_factory=list)
  comp_names : List[str] = dataclasses.field(default_factory=list)
  containing_svar_names : List[str] = dataclasses.field(default_factory=list)
  svars : List[StateVariable] = dataclasses.field(default_factory=list)
  srecs : List[Subrecord] = dataclasses.field(default_factory=list)

  # def __repr__( self ) -> str:
  #   r = reprlib.Repr()
  #   r.maxlevel = 1
  #   return r.repr(self)

  def __eq__(self, other: object) -> bool:
    if isinstance(other, StateVariable):
      same = True
      same = same and (self.name == other.name)
      same = same and (self.title == other.title)
      same = same and (self.agg_type == other.agg_type)
      same = same and (self.data_type == other.data_type)
      same = same and (self.list_size == other.list_size)
      same = same and (self.order == other.order)
      same = same and (self.dims == other.dims)
      same = same and (self.comp_names == other.comp_names)
      same = same and (self.containing_svar_names == other.containing_svar_names)
      return same
    return False

  @property
  def recursive_names(self) -> List[str]:
    """Recusively gather the names of all svars all the way down"""
    return [self.name] + flatten([sv.recursive_names for sv in self.svars])

  @property
  def recursive_svars(self) -> List[StateVariable]:
    """Recusively gather the nested svars all the way down"""
    return [self] + flatten([sv.recursive_svars for sv in self.svars])

  @property
  def comp_layout(self) -> List[List[str]]:
    """Get layout of component state variables for this svar."""
    comp_layout: List[List[str]] = []
    if self.agg_type == StateVariable.Aggregation.SCALAR:
      comp_layout.append( [self.name] )
    elif self.agg_type == StateVariable.Aggregation.ARRAY:
      comp_layout.append( [self.name] * np.prod(self.dims) )
    elif self.agg_type == StateVariable.Aggregation.VECTOR:
      comp_layout.append( flatten([sv.comp_layout for sv in self.svars if sv != self]) )
    else:
      comp_layout.append( flatten([sv.comp_layout for sv in self.svars if sv != self] * np.prod(self.dims)) )
    return comp_layout

  @property
  def comp_titles(self) -> List[str]:
    return [ ssvar.title for ssvar in self.svars ]

  @property
  def atom_qty( self ) -> int:
    ''' Returns the quantity of atoms (scalars of the contained type) based on the aggregate type. '''
    if self.agg_type == StateVariable.Aggregation.VECTOR:
      return sum( svar.atom_qty for svar in self.svars )
    elif self.agg_type == StateVariable.Aggregation.ARRAY:
      return int( np.prod( self.dims ) )
    elif self.agg_type == StateVariable.Aggregation.VEC_ARRAY:
      return int( np.prod( self.dims ) ) * sum( svar.atom_qty for svar in self.svars )
    return 1

@dataclass(eq=False)
class Subrecord:
  class Org(IntEnum):
    ''' The organization of data in a subrecord is either ordered by variable or object. '''
    RESULT = 0
    OBJECT = 1
    INVALID = -1

  '''   A Subrecord contains a number of state varaibles organized in a certain order. '''
  ## atom variables describe the layout of the svars by their scalar components, e.g. svar_atom_lengths[0] gives the number of scalars in the first svar in the srec
  ## ordinal variables describe the association of the svars in the srec with blocks of locally-numbered (ordinal) mesh entities by ordinal
  name : str = ''
  class_name : str = ''
  superclass : Superclass = Superclass.M_QTY_SUPERCLASS
  organization : Org = Org.INVALID
  qty_svars : int = -1
  svar_names : List[str] = dataclasses.field(default_factory=list)

  # Everything after this is calculated by the reader to make queries easier
  # TODO: break this information out into a seperate class as much as possible
  svars : List[StateVariable] = dataclasses.field(default_factory=list)
  svar_atom_lengths : NDArray[np.int64] = field(default_factory=lambda: np.empty([0],dtype = np.int64))
  svar_atom_offsets : NDArray[np.int64] = field(default_factory=lambda: np.empty([0],dtype = np.int64))
  svar_comp_layout : List[List[str]] = field(default_factory=list)
  svar_comp_offsets : List[NDArray[np.integer]] = field(default_factory=list)
  ordinal_blocks : NDArray[np.int64] = field(default_factory=lambda: np.empty([0], dtype = np.int64))
  ordinal_block_counts : NDArray[np.int64] = field(default_factory=lambda: np.empty([0], dtype = np.int64))
  ordinal_block_offsets : NDArray[np.int64] = field(default_factory=lambda: np.empty([0], dtype = np.int64))
  state_byte_offset : int = -1
  atoms_per_label : int = -1
  byte_size : int = 0
  # for each svar in the srec, the offset
  svar_ordinal_offsets : NDArray[np.int64] = field(default_factory=lambda: np.empty([0], dtype = np.int64))
  svar_byte_offsets : NDArray[np.int64] = field(default_factory=lambda: np.empty([0], dtype = np.int64))
  srec_fmt_id: int = 0

  def __eq__(self, other: object) -> bool:
    if isinstance(other, Subrecord):
      same = True
      same = same and (self.name == other.name)
      same = same and (self.class_name == other.class_name)
      same = same and (self.superclass == other.superclass)
      same = same and (self.organization == other.organization)
      same = same and (self.qty_svars == other.qty_svars)
      same = same and (self.svar_names == other.svar_names)
      same = same and (self.ordinal_blocks == other.ordinal_blocks).all()
      return same
    return False

  def scalar_svar_coords(self, aggregate_match: str, scalar_svar_name: str) -> NDArray[np.integer]:
    coords = []
    for idx, (svar_name, svar_comps) in enumerate( zip(self.svar_names, self.svar_comp_layout) ):
      matches = [ ( idx, self.svar_comp_offsets[idx][jdx] ) for jdx, svar in enumerate(svar_comps) if svar == scalar_svar_name ]
      if ( svar_name == aggregate_match or aggregate_match == "" ) and len( matches ) > 0:
        coords.append( matches )
    # flatten coords
    coords = [[item for sublist in coords for item in sublist]]
    return np.array( *coords )

  def calculate_memory_offsets(self,
                               match_aggregate_svar: str,
                               svars_to_query: List[StateVariable],
                               ordinals: NDArray[np.integer],
                               matching_int_points: Dict[str,Dict[str,List[int]]],
                               array_indices: Dict[str,List[int]]) -> Tuple[NDArray[np.int64],NDArray[np.int32]]:
    """Calculate the memory offsets into the subrecord for the passed in ordinals"""
    #  determine if any of the queried svar components are in the StateRecord.. and extract only the comps we're querying for
    # col 0 is supposed to be the svar, col 2 is supposed to be the comp in the svar for agg svars
    qd_svar_comps = np.empty([0,2],dtype=np.int64)
    # TODO : remove special stress/strain handling when dyna/diablo aggregate them appropriately
    match_aggregate_svar = '' if match_aggregate_svar in ('stress','strain') else match_aggregate_svar
    svar: StateVariable
    for svar in svars_to_query:
      svar_coords = self.scalar_svar_coords( match_aggregate_svar, svar.name )
      # Special handling for integration point data
      ipts = matching_int_points.get( svar.name, {} ).get( self.name, [] )
      if len( ipts ) > 0:
        svar_coords = svar_coords[ ipts ]
      # Special handling for array data
      arr_idxs = array_indices.get(svar.name, [])
      if len(arr_idxs) > 0:
        svar_coords = svar_coords[arr_idxs]
      qd_svar_comps = np.concatenate( ( qd_svar_comps, svar_coords ), axis = 0 )

    # at this point we use the StateRecord mo_blocks to find which block each label belongs to using a bining algorithym from numpy
    #   we have [start1, stop1, start2, stop2, etc], but digitize operates as [ start1, start2, start3, etc...] where start2 = stop1
    #   can't just ::2 the blocks and use that to digitize, since we need to exclude labels falling OUTSIDE the upper limit
    # this is currently where basically ALL the time cost for calculating labels/indices for a query
    #   and there isn't much to do to improve performance at the python level unless there is a better numpy/pandas algo to apply
    #   all labels will be in bins, but only odd # bins are actual bins, even bins are between the block ranges so mask to only grab rows with odd bins

    # *** THE ORDINAL BLOCKS MUST BE MONOTONICALLY INCREASING FOR THIS TO WORK ***
    #  since they denote local ordinals and define the srec label layout this *should* always be the case
    # ... and yes this things falling into the first bin return index 1, essentially they return the first index greater than them
    blocks_of_ordinals = np.digitize( ordinals, self.ordinal_blocks )

    # for each ordinal we have, which block does that ordinal fall into from the set of blocks the srec has
    # a mask that is true only for odd-index blocks, which are those defining the ranges of ordinals that belong to the srec
    # this is the same size as srec_ordinal_bins and ordinals
    in_srec = ( blocks_of_ordinals & 0x1 ).astype( bool )

    # each ordinal for queried labels that is in the subrecord, using these
    #  to index the label set for the class should give the correct set of labels, in the correct order
    #  for the result
    ordinals_in_srec: NDArray[np.int32] = ordinals[ in_srec ]

    # for each ordinal that is in the subrecord, which block is it in
    # this gives the index into the list of blocks (for the current srec) that the lower bound of the block range is located at
    indices_of_blocks_of_ordinals_in_srec = blocks_of_ordinals[ in_srec ] - 1 # subtract one to get the index of the starting ordinal of the block instead of ending ordinal

    # for each ordinal in the srec, subtract starting range of that block
    # for each ordinal this gives the location of that ordinal in the block that it lies in
    block_local_indices_of_ordinals_in_srec = (ordinals_in_srec - self.ordinal_blocks[ indices_of_blocks_of_ordinals_in_srec ])

    # take the location of each ordinal relative to the block it lies in and add the cumsum of the block counts of all previous blocks in the srec
    # to give the location of the ordinal when the blocks are densely-packed to form the srec
    dense_index_of_ordinals_in_srec = block_local_indices_of_ordinals_in_srec + self.ordinal_block_offsets[ (indices_of_blocks_of_ordinals_in_srec // 2) ]

    # this relates the mesh entities to the srec as it will be laid out in memory, we still need to get only the svars we care about from the srec,
    # so additional components for each of the above mesh-related indices is required
    # this latter step might be able to be simplified since the set of srec-local svar-offsets to be queried for each mesh entity associated with
    # the srec is identical for object ordering. result ordering complicates things and is why we handle both cases by calculating all the srec-local
    # memory locations we'll be pulling data from

    srec_memory_offsets = np.empty( [ dense_index_of_ordinals_in_srec.size, qd_svar_comps.shape[0] ], dtype = np.int64 )
    # we set the first column of the memory offsets we'll be querying for this srec for this query to the dense indices of the mesh entity locations
    srec_memory_offsets[:,:] = 0
    srec_memory_offsets[:,0] = dense_index_of_ordinals_in_srec

    # we then loop backward over the set of individual memory locations (where we pull the atomic values from, often scalars but also can be arrays, regardless
    #  they're the smallest unit we deal with in the database, hence "atoms"), calculating the atom offsets from the base label offset
    #  this is easier to understand in the object-ordered case, where the mesh-entity dense location is multiplied by the total size of the srec svars for each label
    #  that gives us the actual memory offset of the data for this particular mesh-entity, then we add the offset for the specific state-variable, and
    #  finally for the specific term in that state variable ( for aggregate state variables, otherwise that last offset is 0 ( for all scalar terms especially ))

    # we work from the last column / svar to the first the first contains the ordinal info needed to compute the rest (in the RESULT organization case.. so we just do the same in both cases)
    if self.organization == Subrecord.Org.OBJECT:
      for idx_col, comp in zip( srec_memory_offsets.T[::-1,:], qd_svar_comps[::-1,:]):
        idx_col[:] = srec_memory_offsets[:,0] * self.atoms_per_label + self.svar_atom_offsets[ comp[0] ] + comp[1]  # type: ignore
    elif self.organization == Subrecord.Org.RESULT:
      for idx_col, comp in zip( srec_memory_offsets.T[::-1,:], qd_svar_comps[::-1,:]):
        idx_col[:] = self.svar_atom_offsets[ comp[0] ] * self.total_ordinal_count + self.svar_atom_lengths[ comp[0] ] * srec_memory_offsets[:,0] + comp[1]  # type: ignore

    return srec_memory_offsets, ordinals_in_srec

  # right now we're assuming we have the ordinals, we could add bounds checking as a debug version, or auto-filter for parallel versions
  # we're also assuming the write_data is filtered appropriately so that only the data being written out to this procs database is included
  def extract_ordinals(self, buffer: bytes, ordinals: NDArray[np.integer],
                       write_data: Optional[ArrayLike] = None ) -> Tuple[NDArray[Any],bytes]:
    # TODO(wrt): check that buffer is appropriately sized
    all_same = self.organization == Subrecord.Org.OBJECT or all( svar.data_type == self.svars[0].data_type for svar in self.svars )
    if all_same:
      svar_idx = 0
      buffer_slice = (0 , len(buffer))
    else:
      # identify which svar the labels are indexing into (we only query a single svar at a time)
      svar_idxs = np.digitize( ordinals, self.svar_ordinal_offsets )
      # assert all labels are from the same svar
      assert( np.min(svar_idxs) == np.max(svar_idxs) )
      svar_idx = int(np.min(svar_idxs) - 1)
      # modify the ordinals to offset in the svar for this srec
      ordinals = ordinals - self.svar_ordinal_offsets[svar_idx]
      # only read the portion of the subrecord related to this svar
      buffer_slice = (self.svar_byte_offsets[svar_idx], self.svar_byte_offsets[svar_idx+1])
    var_data: NDArray[Any] = np.frombuffer( buffer[ buffer_slice[0] : buffer_slice[1] ], dtype = self.svars[svar_idx].data_type.numpy_dtype() )
    if write_data is not None:
      var_data = var_data.copy( )
      var_data[ ordinals ] = write_data
      write_buffer = bytearray(buffer)
      write_buffer[ buffer_slice[0] : buffer_slice[1] ] = var_data.tobytes()
      buffer = bytes(write_buffer)
    return var_data[ ordinals ], buffer

  @property
  def total_ordinal_count( self ) -> int:
    return max( 1, int(np.sum( self.ordinal_block_counts )) )

  def struct_repr(self) -> Tuple[Union[str,List[str]],bool]:
    '''
    This function returns the string representation of a Subrecord, to be used when interpreting the bytes during reading of state.
    '''
    if self.organization == Subrecord.Org.OBJECT:
      # if subrecord is Object ordered there will only be 1 data type, so just pull the datatype of the first svar
      return str(self.atoms_per_label * self.total_ordinal_count) + self.svars[0].data_type.struct_repr(), True
    else:
      # if all the svars are the same type, merge the struct repr into one to make querying easier
      base_repr = self.svars[0].data_type.struct_repr()
      if ( all( base_repr == svar.data_type.struct_repr() for svar in self.svars ) ):
        return str(self.atoms_per_label * self.total_ordinal_count) + self.svars[0].data_type.struct_repr(), True
      else:
        warnings.warn( "Querying data from a mixed-type subrecord is currently unsupported." )
        return [str(svar.atom_qty * self.total_ordinal_count) + svar.data_type.struct_repr() for svar in self.svars ], False

@dataclass
class MeshObjectClass:
  """Dataclass storing basic information about each Mesh Element Class."""
  mesh_id : int = 0
  short_name : str = ''
  long_name : str = ''
  sclass : Superclass = Superclass.M_QTY_SUPERCLASS
  elem_qty : int = 0
  idents_exist: bool = True


@dataclass
class StateMap:
  file_number : int = -1
  file_offset : int = -1
  time : float = -1.0
  state_map_id : int = -1
