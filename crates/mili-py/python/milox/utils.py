"""milox ``utils`` — the pure result-reshaping subset of upstream
``mili.utils``.

``result_dictionary_to_dataframe`` / ``query_data_to_dataframe`` are
verbatim ports (decision 18 — non-parity post-processing over the
already-parity-correct primal/derived ``QueryDict``; pure pandas
reshaping). Ported line-for-line from
``reference/mili-python/src/mili/utils.py`` (only the ``mili.*``
imports repointed at ``milox``).
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional, Union

import numpy as np
import pandas as pd
from numpy.typing import ArrayLike, NDArray

from .reductions import combine


def argument_to_ndarray(
    argument: ArrayLike, dtype: Any
) -> Optional[NDArray[Any]]:
    """Verbatim port of upstream ``mili.utils.argument_to_ndarray``.

    Convert an ArrayLike object into a numpy array of the specified
    dtype, returning ``None`` if the conversion raises."""
    try:
        if np.isscalar(argument):
            as_array = np.array([argument], dtype=dtype)
        else:
            as_array = np.array(argument, dtype=dtype)
    except:  # noqa: E722 — upstream bare except
        as_array = None
    return as_array


def results_by_element(result_dict):  # noqa: ANN001,ANN201
  """Verbatim port of upstream ``mili.utils.results_by_element``
  (decision 18/19 — pure non-parity result reshaping over the
  already-parity-correct ``QueryDict``)."""
  if not isinstance(result_dict, dict):
    result_dict = combine(result_dict)

  reorganized_data = {}
  for svar in result_dict:
    if svar not in reorganized_data:
      reorganized_data[svar] = {}

    if isinstance( result_dict[svar], pd.DataFrame ):
      raise ValueError("The results_by_element function does not support Pandas Dataframes.")

    for elem_idx, element in enumerate(result_dict[svar]['layout']['labels']):
      if element not in reorganized_data:
        reorganized_data[svar][element] = result_dict[svar]['data'][:,elem_idx,:]

  return reorganized_data


def writeable_from_results_by_element(results_dict, results_by_element):  # noqa: ANN001,ANN201
  """Verbatim port of upstream
  ``mili.utils.writeable_from_results_by_element`` (decision 18/19)."""
  if not isinstance(results_dict, dict):
    results_dict = combine(results_dict)
  for result in results_by_element:
    if result in results_dict and results_dict[result]['data'].size > 0:
      result_shape = results_dict[result]['data'][:,0].shape
      for idx, element in enumerate(list(results_by_element[result].keys())):
          write_data = np.array(results_by_element[result][element])
          if write_data.shape != result_shape:
            write_data = np.reshape(write_data, result_shape)
          results_dict[result]['data'][:,idx] = write_data
  return results_dict


def query_data_to_dataframe(
    data: NDArray[np.floating],
    states: NDArray[np.int32],
    labels: NDArray[np.int32],
) -> pd.DataFrame:
    """Creates a Pandas DataFrame from a 3d NumPy array returned by MiliDatabase.Query."""
    if data.ndim != 3:
        raise ValueError("'data' Array must be 3-dimensional")
    if states.ndim != 1:
        raise ValueError("'states' Array must be 1-dimensional")
    if labels.ndim != 1:
        raise ValueError("'labels' Array must be 1-dimensional")
    if data.shape[0] != states.shape[0]:
        raise ValueError("Mismatch between shape of states and data.")

    if data.shape[2] == 1:
        df = pd.DataFrame(
            data.reshape(data.shape[:-1]), columns=labels, index=states
        )
    else:
        df = pd.DataFrame.from_records(data)  # type: ignore
        df.index = states  # type: ignore
        df.columns = labels  # type: ignore

    return df


def result_dictionary_to_dataframe(
    result_dict: Union[Dict[str, dict], List[Dict[str, dict]]],
) -> Dict[str, pd.DataFrame]:
    """Convert dictionary from default format of MiliDatabase.query method to a Pandas DataFrame.

    NOTE: This transformation loses some of the information stored in the result_dictionary. The
          times, class_name, source, title and components in the original result dictionary are
          not transferred.
    """
    result_dataframes = {}

    if isinstance(result_dict, list):
        result_dict = combine(result_dict)

    for svar_name, svar_result_dict in result_dict.items():
        if isinstance(svar_result_dict, pd.DataFrame):
            result_dataframes[svar_name] = svar_result_dict
        else:
            df = pd.DataFrame()
            if svar_result_dict["data"].size > 0:
                data = svar_result_dict["data"]
                states = svar_result_dict["layout"]["states"]
                labels = svar_result_dict["layout"]["labels"]
                df = query_data_to_dataframe(data, states, labels)
            result_dataframes[svar_name] = df

    return result_dataframes
