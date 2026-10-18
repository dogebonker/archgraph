defmodule Demo.Worker do
  alias Demo.A, as: Entry
  def run, do: Entry.run()
end
