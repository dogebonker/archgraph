# Parse source only: never load Mix, expand macros or evaluate the scanned project.
defmodule ArchgraphElixir do
  def main do
    [root] = System.argv()

    input =
      case IO.read(:stdio, :eof) do
        :eof -> ""
        value -> value
      end

    files = String.split(input, <<0>>, trim: true)

    trees =
      Enum.map(files, fn file ->
        source = File.read!(Path.join(root, file))

        {file, Code.string_to_quoted!(source, file: file, columns: true)}
      end)

    initial = %{modules: %{}, records: []}

    indexed =
      Enum.reduce(trees, initial, fn {file, tree}, acc ->
        {_, acc} = walk(tree, context(file, %{}, %{}, true), acc)
        acc
      end)

    external = external_modules()

    result =
      Enum.reduce(trees, %{indexed | records: []}, fn {file, tree}, acc ->
        {_, acc} = walk(tree, context(file, indexed.modules, external, false), acc)
        acc
      end)

    imports = Enum.reverse(result.records)

    edges =
      for record <- imports, to <- Map.get(record, :targets, []), record.from != to do
        %{
          from: record.from,
          to: to,
          kind: record.kind,
          line: record.line,
          specifier: record.specifier
        }
      end
      |> Enum.uniq_by(&{&1.from, &1.to, &1.kind})
      |> Enum.sort_by(&{&1.from, &1.to, &1.kind})

    IO.write(
      json(%{files: files, edges: edges, imports: imports, elixir_version: System.version()})
    )
  end

  defp context(file, modules, external, indexing) do
    %{
      file: file,
      current: nil,
      aliases: %{},
      modules: modules,
      external: external,
      indexing: indexing,
      kind: "runtime"
    }
  end

  defp external_modules do
    # Only runtime code-path application manifests establish package ownership.
    for path <- :code.get_path(),
        appfile <- Path.wildcard(Path.join(List.to_string(path), "*.app")),
        {:ok, terms} = :file.consult(String.to_charlist(appfile)),
        {:application, app, properties} <- terms,
        module <- Keyword.get(properties, :modules, []),
        into: %{} do
      {Atom.to_string(module), Atom.to_string(app)}
    end
  end

  defp module_name({:archgraph_module, name}, _), do: name
  defp module_name({:__MODULE__, _, _}, ctx), do: ctx.current

  defp module_name({:__aliases__, _, parts}, ctx) do
    case parts do
      [{:__MODULE__, _, _} | rest] when not is_nil(ctx.current) ->
        if Enum.all?(rest, &is_atom/1),
          do: Enum.join([ctx.current | Enum.map(rest, &Atom.to_string/1)], ".")

      _ ->
        if parts != [] && Enum.all?(parts, &is_atom/1) do
          case parts do
            [:"Elixir" | rest] ->
              "Elixir." <> Enum.map_join(rest, ".", &Atom.to_string/1)

            [first | rest] ->
              prefix =
                Map.get(ctx.aliases, Atom.to_string(first), "Elixir." <> Atom.to_string(first))

              Enum.join([prefix | Enum.map(rest, &Atom.to_string/1)], ".")
          end
        end
    end
  end

  defp module_name(atom, _) when is_atom(atom) and atom not in [nil, true, false],
    do: Atom.to_string(atom)

  defp module_name(_, _), do: nil

  defp declaration(node, ctx) do
    case node do
      {:__aliases__, _, [:"Elixir" | _]} ->
        module_name(node, ctx)

      {:__aliases__, _, [first | _]} when is_atom(first) ->
        name = module_name(node, ctx)

        if name && ctx.current && !Map.has_key?(ctx.aliases, Atom.to_string(first)),
          do: ctx.current <> "." <> String.replace_prefix(name, "Elixir.", ""),
          else: name

      _ ->
        module_name(node, ctx)
    end
  end

  defp record(node, meta, ctx, acc) do
    if ctx.indexing do
      acc
    else
      name = module_name(node, ctx)

      base = %{
        from: ctx.file,
        specifier: if(name, do: display(name), else: Macro.to_string(node)),
        line: Keyword.get(meta, :line, 1),
        kind: ctx.kind
      }

      item =
        cond do
          is_nil(name) ->
            Map.put(base, :reason, "dynamic")

          Map.has_key?(ctx.modules, name) ->
            Map.put(base, :targets, Map.fetch!(ctx.modules, name))

          Map.has_key?(ctx.external, name) ->
            Map.put(base, :external, Map.fetch!(ctx.external, name))

          true ->
            Map.put(base, :reason, "missing")
        end

      %{acc | records: [item | acc.records]}
    end
  end

  defp display("Elixir." <> rest), do: rest
  defp display(name), do: ":" <> name

  defp alias_binding(node, options, ctx) do
    case module_name(node, ctx) do
      nil ->
        ctx

      name ->
        binding =
          case Keyword.get(options, :as) do
            {:__aliases__, _, parts} ->
              if Enum.all?(parts, &is_atom/1),
                do:
                  parts
                  |> Enum.map_join(".", &Atom.to_string/1)
                  |> String.replace_prefix("Elixir.", "")

            nil ->
              name |> String.split(".") |> List.last()

            _ ->
              nil
          end

        if binding, do: %{ctx | aliases: Map.put(ctx.aliases, binding, name)}, else: ctx
    end
  end

  defp walk({:quote, _, _}, ctx, acc), do: {ctx, acc}
  defp walk({:__block__, _, nodes}, ctx, acc), do: sequence(nodes, ctx, acc)

  defp walk({definition, _, [node, body]}, ctx, acc)
       when definition in [:defmodule, :defprotocol] do
    case declaration(node, ctx) do
      nil ->
        {ctx, acc}

      name ->
        acc =
          if ctx.indexing,
            do: %{
              acc
              | modules: Map.update(acc.modules, name, [ctx.file], &Enum.uniq([ctx.file | &1]))
            },
            else: acc

        ctx =
          case {ctx.current, node} do
            {current, {:__aliases__, _, [first | _]}}
            when is_binary(current) and is_atom(first) and first != Elixir ->
              binding = Atom.to_string(first)

              if Map.has_key?(ctx.aliases, binding),
                do: ctx,
                else: %{ctx | aliases: Map.put(ctx.aliases, binding, current <> "." <> binding)}

            _ ->
              ctx
          end

        inside = %{ctx | current: name}
        {_, acc} = walk(Keyword.get(body, :do), inside, acc)
        {ctx, acc}
    end
  end

  defp walk({:alias, _meta, [{{:., _, [prefix, :{}]}, _, names} | rest]}, ctx, acc) do
    options = options(rest)

    Enum.reduce(names, {ctx, acc}, fn suffix, {ctx, acc} ->
      name = module_name(prefix, ctx)

      case {name, suffix} do
        {name, {:__aliases__, _, parts}} when is_binary(name) ->
          full = {:archgraph_module, name <> "." <> Enum.map_join(parts, ".", &Atom.to_string/1)}
          {alias_binding(full, options, ctx), acc}

        _ ->
          {ctx, acc}
      end
    end)
  end

  defp walk({:alias, _meta, [node | rest]}, ctx, acc) do
    {alias_binding(node, options(rest), ctx), acc}
  end

  defp walk({directive, meta, [node | rest]}, ctx, acc)
       when directive in [:import, :require, :use] do
    acc = record(node, meta, ctx, acc)

    ctx =
      if directive == :require && Keyword.has_key?(options(rest), :as),
        do: alias_binding(node, options(rest), ctx),
        else: ctx

    {_, acc} = sequence(rest, ctx, acc)
    {ctx, acc}
  end

  defp walk({:@, _, [{:behaviour, meta, [node]}]}, ctx, acc),
    do: {ctx, record(node, meta, ctx, acc)}

  defp walk({:@, _, [{type, _, args}]}, ctx, acc)
       when type in [:spec, :type, :typep, :opaque, :callback, :macrocallback] do
    {_, acc} = sequence(args, %{ctx | kind: "type"}, acc)
    {ctx, acc}
  end

  defp walk({{:., meta, [receiver, function]}, _, args} = node, ctx, acc)
       when function in [:concat, :safe_concat] do
    if module_name(receiver, ctx) == "Elixir.Module" do
      acc = record(node, meta, ctx, acc)
      {_, acc} = concat_argument(args, ctx, acc)
      {ctx, acc}
    else
      acc = record(receiver, meta, ctx, acc)
      {_, acc} = sequence(args, ctx, acc)
      {ctx, acc}
    end
  end

  defp walk({{:., meta, [receiver, _function]}, _, args}, ctx, acc) when is_list(args) do
    acc = record(receiver, meta, ctx, acc)
    {_, acc} = sequence(args, ctx, acc)
    {ctx, acc}
  end

  defp walk({:%, meta, [node, fields]}, ctx, acc) do
    acc = record(node, meta, ctx, acc)
    {_, acc} = walk(fields, ctx, acc)
    {ctx, acc}
  end

  defp walk({:__aliases__, meta, _} = node, ctx, acc) do
    name = module_name(node, ctx)

    # Bare aliases can be ordinary atoms (ETS names); only proven modules count as data references.
    if ctx.kind == "type" || Map.has_key?(ctx.modules, name) || Map.has_key?(ctx.external, name),
      do: {ctx, record(node, meta, ctx, acc)},
      else: {ctx, acc}
  end

  defp walk({:__MODULE__, meta, _} = node, ctx, acc), do: {ctx, record(node, meta, ctx, acc)}

  defp walk({_name, _meta, args}, ctx, acc) when is_list(args) do
    {_, acc} = sequence(args, ctx, acc)
    {ctx, acc}
  end

  defp walk(list, ctx, acc) when is_list(list) do
    {_, acc} = sequence(list, ctx, acc)
    {ctx, acc}
  end

  defp walk({key, value}, ctx, acc) do
    {_, acc} = walk(key, ctx, acc)
    {_, acc} = walk(value, ctx, acc)
    {ctx, acc}
  end

  defp walk(_, ctx, acc), do: {ctx, acc}

  # Module.concat arguments are namespace atoms, while calls computing them still execute.
  defp concat_argument({alias, _, _}, ctx, acc) when alias in [:__aliases__, :__MODULE__],
    do: {ctx, acc}

  defp concat_argument(list, ctx, acc) when is_list(list),
    do: Enum.reduce(list, {ctx, acc}, fn node, {ctx, acc} -> concat_argument(node, ctx, acc) end)

  defp concat_argument(node, ctx, acc), do: walk(node, ctx, acc)

  defp sequence(nodes, ctx, acc),
    do: Enum.reduce(nodes, {ctx, acc}, fn node, {ctx, acc} -> walk(node, ctx, acc) end)

  defp options([options]) when is_list(options), do: options
  defp options(_), do: []

  # Avoid a project JSON dependency and retain support for OTP versions before :json.
  defp json(value) when is_map(value),
    do:
      "{" <>
        Enum.map_join(Enum.sort(value), ",", fn {k, v} -> json(to_string(k)) <> ":" <> json(v) end) <>
        "}"

  defp json(value) when is_list(value), do: "[" <> Enum.map_join(value, ",", &json/1) <> "]"

  defp json(value) when is_binary(value) do
    escaped =
      for <<char::utf8 <- value>>, into: "" do
        case char do
          ?" ->
            "\\\""

          ?\\ ->
            "\\\\"

          char when char < 32 ->
            "\\u" <> (char |> Integer.to_string(16) |> String.pad_leading(4, "0"))

          char ->
            <<char::utf8>>
        end
      end

    "\"" <> escaped <> "\""
  end

  defp json(value) when is_integer(value), do: Integer.to_string(value)
end

ArchgraphElixir.main()
