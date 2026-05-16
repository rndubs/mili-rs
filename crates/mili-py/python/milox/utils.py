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

from typing import Dict, List, Union

import numpy as np
import pandas as pd
from numpy.typing import NDArray

from .reductions import combine


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
